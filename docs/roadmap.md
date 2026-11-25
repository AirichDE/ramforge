# Roadmap

## 0.1 – Fundament (aktuell)
- [x] Kern: 11 Testverfahren, Multi-Thread-Runner, gesperrter Speicher, Selbsttest
- [x] Benchmark: Bandbreite (Lesen/Schreiben/Kopieren), Latenz L1 bis DRAM
- [x] System: CPUID, SMBIOS (Board, BIOS, Module mit Takt und Spannung)
- [x] Sensoren: Windows-intern, HWiNFO Shared Memory, CSV mit Fehlermarkern
- [x] SPD-Decoder DDR4/DDR5 (JEDEC, XMP/EXPO-Erkennung, DDR4-XMP-Profile experimentell)
- [x] GUI: Tabs, Sensor-Seitenleiste, Hell/Dunkel, Tastenkürzel, Screenshot, Berichte
- [x] Portable EXE, statisch gelinkt
- [x] GPL-3.0-Volltext in `LICENSE`
- [x] App-Icon und Logo aus `assets/logo/`
- [x] Signierte Update-Prüfung (Ed25519 + SHA-256), Selbstaustausch der EXE
- [x] Neuigkeiten in der App, Opt-in-Statistik mit Widerruf
- [x] `RAMForge-USB.exe` als Gerüst (Laufwerksliste, Update-Dialog), `ramforge-sign`
- [ ] Update-Kette einmal end-to-end gegen den echten Server testen (sobald DNS aktiv ist)

## 0.2 – Boot-Variante
- [x] Debian-12-Live (live-build), UEFI + BIOS, Oberfläche startet automatisch (cage, ohne Anmeldung)
- [x] Secure Boot: signierter Shim/GRUB/Kernel von Debian, getestet in QEMU mit Microsoft-Schlüsseln
- [x] Bootmenü mit RAMForge-Branding, Grafik-Sicherheitsmodus, Memtest86+ (ohne Secure Boot), UEFI-Einstellungen
- [x] Zwei Varianten: Voll (Grafiktreiber, Vulkan, GPU-Test) und Kompakt (RAM-Test, Sensoren, CPU/RAM)
- [x] Sensor- und SPD-Treiber werden geladen (i2c-dev, ee1004, spd5118, jc42, k10temp, coretemp, nct6775)
- [ ] hwmon- und SPD-Werte auf echter Hardware prüfen
- [ ] Physische Adressen in Fehlermeldungen (`/proc/self/pagemap`), daraus DIMM/Kanal ableiten
- [ ] ISO-Build in CI

## 0.3 – USB-Werkzeug, Benchmarks
- [x] `RAMForge-USB.exe`: bootfähig einrichten (MBR + FAT32, Kopie mit Prüfung, LÖSCHEN-Bestätigung)
- [x] Sperren: nur USB, nie System-/Startplatte, nie der eigene Datenträger, nie ein Stick mit `RAMForge-Keys`
- [x] Boot-Dateien ohne Formatieren aktualisieren, Berichte bleiben erhalten
- [x] Portable Windows-Version auf den Stick (`RAMForge\`), gemeinsamer Datenordner für Windows und Boot
- [x] Benchmark-Suite: CPU (Ganzzahl, Gleitkomma, SIMD, geprüft), RAM, CPU+RAM unter Last, Verlauf
- [x] GPU-Benchmark als optionales Modul `RAMForge-GPU.exe` (wgpu: VRAM, FP32, Übertragung)
- [ ] Boot-Abbild über die signierte Update-Kette herunterladen (Assets `boot_iso`, `boot_iso_compact`, `gpu_module`)
- [ ] USB-SSDs erkennen, die sich als feste Laufwerke melden
- [ ] Legacy-BIOS-Start vom FAT32-Stick (bisher nur UEFI; die ISO selbst startet auch per BIOS)

## 0.4 – Hardwarezugriff unter Windows
- [ ] PawnIO-Anbindung: CPU-Temperatur (MSR/SMN), SuperIO (Spannungen, Lüfter), SMBus-SPD, DIMM-Temperaturen
- [ ] Aktuelle Timings aus dem Speichercontroller (Intel MCHBAR, AMD UMC)
- [ ] WHEA-Fehler aus dem Windows-Ereignisprotokoll ohne HWiNFO

## Später
- [ ] XMP-3.0- und EXPO-Profile vollständig dekodieren (Dumps gesucht, siehe CONTRIBUTING)
- [ ] Adressbewusster Row-Hammer-Test (Boot-Variante)
- [ ] Mehrsprachigkeit (Deutsch, Englisch, weitere)
- [ ] Konfigurierbare Testprofile als Datei, Import/Export
- [ ] Vergleich von Benchmarks und Berichten
- [ ] Linux-Desktop-Build (AppImage)
