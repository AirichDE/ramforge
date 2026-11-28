//! GPU benchmark via wgpu (Vulkan / DirectX 12 / Metal / OpenGL ES), vendor
//! independent: VRAM copy bandwidth, FP32 compute throughput and host <-> GPU
//! transfer speed. Every number comes with a check: copied data is compared
//! word by word, compute results against a CPU reference.
//!
//! If no usable GPU or driver exists, `run` returns a result with `skipped`
//! set; it never panics.

#[allow(unused_imports)]
use ramforge_i18n::{t, tf};
use serde::{Deserialize, Serialize};

/// Bumped when a workload changes (scores comparable only with equal version).
pub const WORKLOAD_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GpuResult {
    pub adapter: String,
    pub backend: String,
    pub driver: String,
    pub device_type: String,
    /// Buffer size used for bandwidth tests, bytes.
    pub buffer_bytes: u64,
    /// VRAM -> VRAM copy, counted as bytes read + written per second (GB/s).
    pub vram_gbs: f64,
    /// FP32 fused multiply-add throughput (TFLOPS).
    pub fp32_tflops: f64,
    /// Host -> GPU (GB/s)
    pub upload_gbs: f64,
    /// GPU -> host (GB/s)
    pub download_gbs: f64,
    /// Words that came back wrong (copy check) + compute results off the reference.
    pub errors: u64,
    pub skipped: Option<String>,
}

/// Name of the separate GPU module next to RAMForge.exe / in /usr/local/bin.
#[cfg(windows)]
pub const MODULE_FILE: &str = "RAMForge-GPU.exe";
#[cfg(not(windows))]
pub const MODULE_FILE: &str = "ramforge-gpu";

/// Where the GPU module is installed, if it is.
pub fn find_module() -> Option<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        dirs.push(d);
    }
    dirs.push("/usr/local/bin".into());
    dirs.into_iter()
        .map(|d| d.join(MODULE_FILE))
        .find(|p| p.is_file())
}

/// Runs the GPU module and reads its JSON result. Progress lines on stderr
/// look like "PROGRESS 0.30 GPU · VRAM-Bandbreite".
pub fn run_module(module: &std::path::Path, progress: &dyn Fn(f32, &str)) -> GpuResult {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(module);
    // The module process shows its progress texts in the caller's language.
    cmd.env("RAMFORGE_LANG", ramforge_i18n::code());
    cmd.arg("--json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return GpuResult {
                skipped: Some(tf!(
                    "GPU-Modul startet nicht: {e}",
                    "GPU module does not start: {e}"
                )),
                ..Default::default()
            }
        }
    };
    if let Some(err) = child.stderr.take() {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix("PROGRESS ") {
                let (f, text) = rest.split_once(' ').unwrap_or((rest, ""));
                progress(f.parse().unwrap_or(0.0), text);
            }
        }
    }
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            return GpuResult {
                skipped: Some(tf!("GPU-Modul: {e}", "GPU module: {e}")),
                ..Default::default()
            }
        }
    };
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| GpuResult {
        skipped: Some(tf!(
            "GPU-Modul lieferte kein Ergebnis ({e})",
            "GPU module returned no result ({e})"
        )),
        ..Default::default()
    })
}

#[cfg(feature = "bench")]
mod bench {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    const WG: u32 = 256;
    const ITERS: u32 = 4096;
    const CHAINS: u32 = 8;
    const CHECK: usize = 4096;

    const SHADER: &str = r#"
@group(0) @binding(0) var<storage, read_write> outb: array<f32>;
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) n: vec3<u32>) {
    let i = id.x + id.y * n.x * 256u;
    let s = f32(i % 1024u) * 0.0001;
    // 8 independent FMA chains per thread keep the ALUs busy.
    var v0 = s; var v1 = s + 0.125; var v2 = s + 0.25; var v3 = s + 0.375;
    var v4 = s + 0.5; var v5 = s + 0.625; var v6 = s + 0.75; var v7 = s + 0.875;
    for (var k = 0u; k < 4096u; k = k + 1u) {
        v0 = fma(v0, 0.9999, 0.0001); v1 = fma(v1, 0.9999, 0.0001);
        v2 = fma(v2, 0.9999, 0.0001); v3 = fma(v3, 0.9999, 0.0001);
        v4 = fma(v4, 0.9999, 0.0001); v5 = fma(v5, 0.9999, 0.0001);
        v6 = fma(v6, 0.9999, 0.0001); v7 = fma(v7, 0.9999, 0.0001);
    }
    if (i < arrayLength(&outb)) {
        outb[i] = ((v0 + v1) + (v2 + v3)) + ((v4 + v5) + (v6 + v7));
    }
}
"#;

    /// CPU reference for one invocation of the shader.
    fn reference(i: u32) -> f32 {
        let s = (i % 1024) as f32 * 0.0001;
        let mut v = [0f32; 8];
        for (k, x) in v.iter_mut().enumerate() {
            *x = s + k as f32 * 0.125;
        }
        for _ in 0..ITERS {
            for x in v.iter_mut() {
                *x = x.mul_add(0.9999, 0.0001);
            }
        }
        ((v[0] + v[1]) + (v[2] + v[3])) + ((v[4] + v[5]) + (v[6] + v[7]))
    }

    fn wait(device: &wgpu::Device) -> Result<(), String> {
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(60)),
            })
            .map(|_| ())
            .map_err(|e| tf!("GPU antwortet nicht: {e}", "GPU does not respond: {e}"))
    }

    fn read_back(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &wgpu::Buffer,
        bytes: u64,
    ) -> Result<(Vec<u8>, f64), String> {
        let rb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let t0 = Instant::now();
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        enc.copy_buffer_to_buffer(src, 0, &rb, 0, bytes);
        queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        rb.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        wait(device)?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| tf!("Zurücklesen: {e}", "reading back: {e}"))?;
        let data = rb
            .get_mapped_range(..)
            .map_err(|e| tf!("Zurücklesen: {e:?}", "reading back: {e:?}"))?
            .to_vec();
        let secs = t0.elapsed().as_secs_f64();
        rb.unmap();
        Ok((data, secs))
    }

    fn pattern(words: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(words * 4);
        let mut x: u32 = 0x9E37_79B9;
        for _ in 0..words {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    /// Runs all GPU tests. `progress(fraction, step)` before each step.
    pub fn run(cancel: &AtomicBool, progress: &dyn Fn(f32, &str)) -> GpuResult {
        match pollster::block_on(run_inner(cancel, progress)) {
            Ok(r) => r,
            Err(e) => GpuResult {
                skipped: Some(e),
                ..Default::default()
            },
        }
    }

    async fn run_inner(
        cancel: &AtomicBool,
        progress: &dyn Fn(f32, &str),
    ) -> Result<GpuResult, String> {
        progress(0.0, t!("GPU wird gesucht", "Looking for the GPU"));
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() })
            .await
            .map_err(|_| t!("GPU-Benchmark nicht verfügbar: keine passende Grafikkarte oder kein Treiber (Vulkan/DirectX 12)", "GPU benchmark not available: no suitable graphics card or driver (Vulkan/DirectX 12)").to_string())?;
        let info = adapter.get_info();
        let mut r = GpuResult {
            adapter: info.name.clone(),
            backend: format!("{:?}", info.backend),
            driver: format!("{} {}", info.driver, info.driver_info)
                .trim()
                .to_string(),
            device_type: format!("{:?}", info.device_type),
            ..Default::default()
        };
        if info.device_type == wgpu::DeviceType::Cpu {
            r.skipped = Some(tf!(
                "GPU-Benchmark nicht verfügbar: nur Software-Rendering ({})",
                "GPU benchmark not available: software rendering only ({})",
                info.name
            ));
            return Ok(r);
        }
        let limits = adapter.limits();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("ramforge"),
                required_limits: limits.clone(),
                ..Default::default()
            })
            .await
            .map_err(|e| {
                tf!(
                    "GPU-Benchmark nicht verfügbar: {e}",
                    "GPU benchmark not available: {e}"
                )
            })?;
        let size = (256u64 << 20)
            .min(limits.max_buffer_size)
            .min(limits.max_storage_buffer_binding_size as u64)
            / 4
            * 4;
        r.buffer_bytes = size;
        let words = (size / 4) as usize;
        let host = pattern(words);
        let usage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        let src = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("src"),
            size,
            usage,
            mapped_at_creation: false,
        });
        let dst = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dst"),
            size,
            usage,
            mapped_at_creation: false,
        });

        // Upload: host -> GPU, best of 3.
        progress(
            0.1,
            t!(
                "GPU · Übertragung zur Grafikkarte",
                "GPU · transfer to the graphics card"
            ),
        );
        let mut best = f64::MAX;
        for _ in 0..3 {
            let t0 = Instant::now();
            queue.write_buffer(&src, 0, &host);
            queue.submit([]);
            wait(&device)?;
            best = best.min(t0.elapsed().as_secs_f64());
        }
        r.upload_gbs = size as f64 / best / 1e9;
        if cancel.load(Ordering::Relaxed) {
            return Err("abgebrochen".into());
        }

        // VRAM copy, best of 5 after a warm-up; read + write traffic counted.
        progress(0.3, t!("GPU · VRAM-Bandbreite", "GPU · VRAM bandwidth"));
        let mut best = f64::MAX;
        for round in 0..6 {
            let t0 = Instant::now();
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            for _ in 0..4 {
                enc.copy_buffer_to_buffer(&src, 0, &dst, 0, size);
            }
            queue.submit([enc.finish()]);
            wait(&device)?;
            if round > 0 {
                best = best.min(t0.elapsed().as_secs_f64() / 4.0);
            }
        }
        r.vram_gbs = 2.0 * size as f64 / best / 1e9;

        // Download + verification of the copied data, best of 2.
        progress(
            0.5,
            t!(
                "GPU · Übertragung zum PC und Prüfung",
                "GPU · transfer to the PC and check"
            ),
        );
        let mut best = f64::MAX;
        for _ in 0..2 {
            let (data, secs) = read_back(&device, &queue, &dst, size)?;
            best = best.min(secs);
            r.errors = data
                .chunks_exact(4)
                .zip(host.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count() as u64;
        }
        r.download_gbs = size as f64 / best / 1e9;
        if cancel.load(Ordering::Relaxed) {
            return Err("abgebrochen".into());
        }

        // FP32 compute: 4 FMA chains x ITERS per invocation.
        progress(0.7, t!("GPU · FP32-Rechenleistung", "GPU · FP32 compute"));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fma"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fma"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let max_x = limits.max_compute_workgroups_per_dimension.max(1);
        let groups_total: u32 = 16_384; // 4 Mi invocations
        let (gx, gy) = if groups_total <= max_x {
            (groups_total, 1)
        } else {
            (max_x, groups_total.div_ceil(max_x))
        };
        let invocations = gx as u64 * gy as u64 * WG as u64;
        let out_bytes = (invocations * 4).min(size);
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("out"),
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: out.as_entire_binding(),
            }],
        });
        let mut best = f64::MAX;
        for round in 0..4 {
            let t0 = Instant::now();
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
            queue.submit([enc.finish()]);
            wait(&device)?;
            if round > 0 {
                best = best.min(t0.elapsed().as_secs_f64());
            }
        }
        let flops = invocations as f64 * ITERS as f64 * CHAINS as f64 * 2.0;
        r.fp32_tflops = flops / best / 1e12;
        let (data, _) = read_back(&device, &queue, &out, (CHECK as u64 * 4).min(out_bytes))?;
        for (i, c) in data.chunks_exact(4).enumerate() {
            let got = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            let want = reference(i as u32);
            if !got.is_finite() || (got - want).abs() > 1e-3 * want.abs().max(1.0) {
                r.errors += 1;
            }
        }
        progress(1.0, t!("GPU · fertig", "GPU · done"));
        Ok(r)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn reference_is_stable() {
            assert_eq!(reference(5), reference(5));
            assert!(reference(0).is_finite() && reference(1023).is_finite());
        }

        /// Runs on machines with a GPU; on CI without one it must skip, not fail.
        #[test]
        fn runs_or_skips_cleanly() {
            let r = run(&AtomicBool::new(false), &|_, _| {});
            match &r.skipped {
                Some(why) => assert!(why.contains("nicht verfügbar"), "{why}"),
                None => {
                    assert_eq!(r.errors, 0, "{r:?}");
                    assert!(
                        r.vram_gbs > 1.0
                            && r.fp32_tflops > 0.01
                            && r.upload_gbs > 0.1
                            && r.download_gbs > 0.1,
                        "{r:?}"
                    );
                }
            }
        }
    }
}
#[cfg(feature = "bench")]
pub use bench::run;
