# Builds and tests the RAMForge boot edition (Debian 12 live, Secure Boot).
#
#   .\boot\debian\build.ps1                 # image + app + iso
#   .\boot\debian\build.ps1 -Stage test     # boot tests in QEMU (bios, uefi, uefi-sb, usb-sb)
#   .\boot\debian\build.ps1 -Stage iso -Version 0.3.0
#
# Requirements: Docker Desktop (Linux containers). Output: boot\debian\out\
#   RAMForge-Boot-<version>.iso          full (GPU firmware, Vulkan, GPU module)
#   RAMForge-Boot-Kompakt-<version>.iso  compact (RAM test, sensors, CPU/RAM benchmarks)
#   RAMForge-Boot-RAM-<version>.iso      RAM test only (smallest)
#   (+ .sha256), ramforge-linux, ramforge-gpu-linux, test\*.ppm
param(
    [ValidateSet("all", "image", "app", "iso", "test")]
    [string]$Stage = "all",
    [string]$Version = "0.3.0",
    [string[]]$Tests = @("bios", "uefi", "uefi-sb", "usb-sb"),
    [ValidateSet("full", "compact", "ram")]
    [string[]]$Editions = @("full", "compact", "ram"),
    # Boot test of another image in out\, e.g. RAMForge-Boot-RAM-0.3.0.iso
    [string]$TestIso = ""
)
$ErrorActionPreference = "Continue"   # docker writes progress to stderr
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$boot = (Resolve-Path "$PSScriptRoot").Path
$out = Join-Path $boot "out"
New-Item -ItemType Directory -Force $out | Out-Null
$image = "ramforge-debian-builder:12"
$volume = "ramforge-debian-build"

function Invoke-Builder([string]$script, [string[]]$extra = @(), [string[]]$scriptArgs = @()) {
    $a = @("run", "--rm", "-e", "RAMFORGE_VERSION=$Version",
        "-v", "${root}:/src:ro", "-v", "${boot}:/boot-src:ro", "-v", "${out}:/out", "-v", "${volume}:/build") +
        $extra + @($image, "bash", "/boot-src/scripts/$script") + $scriptArgs
    & docker @a
    if ($LASTEXITCODE -ne 0) { throw "$script fehlgeschlagen (Exit $LASTEXITCODE)" }
}

if ($Stage -in "all", "image") {
    docker build -t $image $boot
    if ($LASTEXITCODE -ne 0) { throw "docker build fehlgeschlagen" }
}
if ($Stage -in "all", "app") { Invoke-Builder "build-app.sh" }
if ($Stage -in "all", "iso") {
    foreach ($e in $Editions) { Invoke-Builder "build-iso.sh" @("--privileged", "-e", "RAMFORGE_EDITION=$e") }
}
if ($Stage -eq "test") {
    $isoArg = if ($TestIso) { @("-e", "ISO=/out/$TestIso") } else { @() }
    foreach ($t in $Tests) { Invoke-Builder "test-boot.sh" (@("--privileged") + $isoArg) @($t) }
}
Get-ChildItem $out -File | Format-Table Name, Length, LastWriteTime
