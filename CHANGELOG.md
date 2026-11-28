# Änderungen / Changelog

Deutsche Fassung zuerst, English version below.

## 0.3.1 – 28.11.2026

### Neu
- **Englisch**: RAMForge, RAMForge-USB und Berichte gibt es jetzt auf Deutsch und Englisch. Windows folgt der Systemsprache, umschaltbar oben rechts (DE | EN), die Wahl wird gespeichert.
- **Boot-Stick startet auf Englisch**; im Startmenü gibt es eigene Einträge auf Deutsch, die Sprache lässt sich auch gleich im ersten Dialog wählen.
- **Neuer Linux-Kernel 6.12** im Boot-System: läuft jetzt auch auf neuen Grafikkarten wie NVIDIA RTX 40, mit aktueller Firmware und Mesa 25.
- **Ausweich-Grafik**: findet Linux keinen passenden Grafiktreiber, startet RAMForge trotzdem (einfache Grafik über den Bildschirmspeicher der Firmware), auch auf echten PCs.
- **Anzeigegröße** einstellbar (Strg + Plus/Minus/0 oder unter „Über“), passt sich auf kleinen Bildschirmen automatisch an.
- **RAM-Hersteller** wird aus der Teilenummer erkannt, wenn das Mainboard keinen meldet.
- **Updates** kommen notfalls auch über GitHub, wenn ramforge.org nicht erreichbar ist (Signatur wird immer geprüft).
- Fehlerbildschirm im Boot-System zeigt Grafikkarte und Treiber, für schnellere Hilfe, auf Deutsch und Englisch.

### Hinweis
- Größen der Boot-Abbilder: *Voll* ≈ 447 MB, *Kompakt* ≈ 365 MB (vorher 250 MB, wegen der Grafik-Firmware für NVIDIA und Intel zum neuen Kernel), *Nur RAM-Test* ≈ 231 MB.

### Behoben
- Boot-System startete auf PCs mit RTX-40-Grafikkarten nicht.
- Kopfleiste verdeckte bei großer Anzeige die Tabs; breite Tabellen waren rechts abgeschnitten (jetzt seitlich scrollbar).
- Haftungsausschluss-Dialog auf kleinen Bildschirmen abgeschnitten.
- Unsinnige Sensorwerte (z. B. −128 °C an unbelegten Anschlüssen) werden ausgeblendet.
- Links („Was ist neu?“, Datenschutz, Download-Seite) öffnen jetzt den Browser; Kopieren und Einfügen funktioniert wieder.
- Ehrlichere Meldung, wenn noch keine Version veröffentlicht ist, statt „Server nicht erreichbar“.
- Copyright-Hinweise aller Linux-Pakete liegen wieder im Boot-System.

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

## 0.3.1 – 2026-11-28

### New
- **English**: RAMForge, RAMForge-USB and the reports are now available in German and English. Windows follows the system language; switch at the top right (DE | EN), the choice is saved.
- **Boot stick starts in English**; the boot menu has German entries, and the language can also be chosen in the very first dialog.
- **New Linux kernel 6.12** in the boot system: now also runs on new graphics cards such as NVIDIA RTX 40, with current firmware and Mesa 25.
- **Fallback graphics**: if Linux finds no suitable graphics driver, RAMForge still starts (simple graphics on the firmware framebuffer), on real PCs too.
- **Display size** adjustable (Ctrl + plus/minus/0 or under "About"), adapts automatically on small screens.
- **RAM manufacturer** is derived from the part number when the mainboard reports none.
- **Updates** also come via GitHub if ramforge.org is unreachable (the signature is always checked).
- The boot error screen shows the graphics card and driver, for faster help, in German and English.

### Note
- Boot image sizes: *Full* ≈ 447 MB, *Compact* ≈ 365 MB (was 250 MB, because of the NVIDIA and Intel graphics firmware for the new kernel), *RAM test only* ≈ 231 MB.

### Fixed
- Boot system did not start on PCs with RTX 40 graphics cards.
- Top bar covered the tabs at large display sizes; wide tables were cut off on the right (now scroll sideways).
- Disclaimer dialog cut off on small screens.
- Nonsense sensor values (e.g. −128 °C on unused inputs) are hidden.
- Links ("What's new?", privacy, download page) open the browser again; copy and paste works again.
- Clearer message when no version has been published yet, instead of "server not reachable".
- Copyright notices of all Linux packages are included in the boot system again.

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
