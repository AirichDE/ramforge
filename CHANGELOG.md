# Änderungen / Changelog

Deutsche Fassung zuerst, English version below.

## 0.3.0 – 25.11.2026

**Erste öffentliche Version.**

### Neu
- **Bootfähiger Stick** (Debian 12 Live, startet auch mit Secure Boot, UEFI und BIOS):
  RAMForge startet direkt nach dem Booten im Vollbild, ohne Anmeldung und ohne Desktop.
- **Drei Boot-Varianten** zur Auswahl im USB-Werkzeug:
  - *Voll* (≈ 343 MB): alles, zusätzlich Grafiktreiber und GPU-Test
  - *Kompakt* (≈ 250 MB): RAM-Test, Sensoren, CPU-/RAM-Benchmarks
  - *Nur RAM-Test* (≈ 215 MB): RAM-Test, Sensoren und SPD, ohne Benchmarks und ohne Netzwerk
- **RAMForge-USB.exe**: richtet einen Stick bootfähig ein (MBR + FAT32, Kopie mit Prüfung),
  legt die Windows-Version dazu und aktualisiert vorhandene Sticks ohne Formatieren.
  Gesperrt sind System- und Startplatten, nicht-USB-Datenträger und Sticks mit Schlüssel-Ordner.
- **Benchmarks**: CPU (Ganzzahl, Gleitkomma, SIMD), RAM-Bandbreite und Latenz,
  CPU+RAM unter Last mit Ergebnisprüfung, Verlauf zum Vergleichen.
- **GPU-Benchmark** als optionales Zusatzmodul (RAMForge-GPU.exe, ≈ 6 MB): VRAM, FP32, Übertragung.
- **Sensor-Aufzeichnung** während Test und Benchmark mit Diagrammen, Phasen- und Fehlermarken, CSV-Export.
- **Signierte Updates**: Ed25519-Signatur und SHA-256-Prüfsumme, Download von GitHub mit Spiegel auf ramforge.org.

### Behoben
- Boot-System: Oberfläche startet jetzt zuverlässig (eigener Benutzer statt root, XWayland), Vollbild ohne Rand.
- Boot-System zeigt Linux-Hinweise statt Windows-Hinweisen.
- ISO-Abbilder rund 40 MB kleiner (Startdateien lagen doppelt auf dem Medium).
- Benchmark-Zeitmessung korrigiert (zu hohe Bandbreiten).
- Einstellungen mit BOM werden wieder gelesen.

### Bekannt
- Die Programme sind vorerst nur auf Deutsch. Englisch mit automatischer Erkennung und Umschalter folgt mit dem nächsten Update.
- Sensor- und SPD-Werte im Boot-System sind auf echter Hardware noch wenig erprobt. Rückmeldungen an report@ramforge.org helfen.

---

## English

## 0.3.0 – 2026-11-25

**First public release.**

### New
- **Bootable stick** (Debian 12 live, works with Secure Boot, UEFI and BIOS): RAMForge starts
  full-screen right after booting, no login, no desktop.
- **Three boot variants**, selectable in the USB tool: *Full* (≈ 343 MB, adds GPU drivers and GPU test),
  *Compact* (≈ 250 MB, RAM test, sensors, CPU/RAM benchmarks), *RAM test only* (≈ 215 MB, no benchmarks, no network).
- **RAMForge-USB.exe**: makes a stick bootable (MBR + FAT32, verified copy), adds the Windows version,
  updates existing sticks without formatting. System/boot disks, non-USB disks and sticks holding a key folder are refused.
- **Benchmarks**: CPU (integer, floating point, SIMD), RAM bandwidth and latency, CPU+RAM under load with result checks, history.
- **GPU benchmark** as an optional module (RAMForge-GPU.exe, ≈ 6 MB): VRAM, FP32, transfers.
- **Sensor logging** during tests and benchmarks with charts, phase and error markers, CSV export.
- **Signed updates**: Ed25519 signature and SHA-256 checksum, download from GitHub with a mirror on ramforge.org.

### Fixed
- Boot system: the interface now starts reliably (own user instead of root, XWayland), full-screen without borders.
- Boot system shows Linux hints instead of Windows hints.
- ISO images about 40 MB smaller (boot files were stored twice).
- Benchmark timing corrected (bandwidth was too high).
- Settings saved with a BOM are read again.

### Known
- The programs are German-only for now. English with automatic detection and a language switch follows in the next update.
- Sensor and SPD readings in the boot system have seen little real hardware yet. Feedback to report@ramforge.org helps.
