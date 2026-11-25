# RAMForge

**Portabler RAM-Test, Benchmark und Hardware-Monitor. Freie Software (GPL-3.0-or-later).**
*Portable memory tester, benchmark and hardware monitor. Free software.*

> Status: Version 0.3 (Vorabversion). Windows-EXE, bootfähiges Abbild (ISO) und
> USB-Werkzeug sind da. Was noch fehlt, steht in [docs/roadmap.md](docs/roadmap.md).
> Website: <https://ramforge.org> · Quellcode: <https://github.com/airichde/ramforge>

RAMForge gibt es in zwei Varianten, die denselben Kern nutzen:

| | Windows-EXE (portabel) | Boot-Variante (USB/ISO, ohne Windows) |
|---|---|---|
| Start | Doppelklick, keine Installation | per BIOS/UEFI vom USB-Stick |
| RAM-Abdeckung | freier RAM (Windows braucht einige hundert MB) | fast der gesamte RAM, plus Memtest86+ für 100 % |
| Sensoren | Windows-intern, alle Sensoren über HWiNFO Shared Memory | alle Sensoren über Linux-Treiber |
| SPD / XMP / EXPO | aus Dump-Datei | direkt vom Modul |
| Start-Schutz | – | startet mit **Secure Boot an** (signierter Shim von Debian) |
| Status | **läuft** | **Vorabversion 0.3** |

## Funktionen (Windows-EXE, Version 0.1)

- **RAM-Test** mit 11 Verfahren: eigene Adresse, Moving Inversions (0/1, 8-Bit, Zufall),
  Zufallsfolge, Zufallszugriff, Block verschieben, wandernde Bits, Modulo 20,
  Row Hammer (experimentell), Bit Fade. Profile *Schnell*, *Standard*, *Extrem* und
  *Bit Fade*, oder eine eigene Auswahl. Mehrere Threads, gesperrter Speicher
  (`VirtualLock`, kein Auslagern), endlose Durchläufe möglich.
- **Fehleranalyse**: Adresse, Soll, Ist, XOR, Bit-Histogramm über alle 64 Datenbits.
- **Selbsttest**: speist einen künstlichen Bitfehler ein und weist damit nach, dass
  Erkennung, Protokoll und Bericht funktionieren.
- **Benchmark**: Lese-, Schreib- und Kopierbandbreite über alle Kerne, Latenz von
  L1 bis DRAM (Pointer Chasing).
- **System**: CPU (CPUID), Mainboard, BIOS und jedes Speichermodul aus SMBIOS
  (Größe, Typ, Nenn- und Ist-Takt, Spannung, Hersteller, Teilenummer, Ranks, ECC).
- **Live-Sensoren** in einer Seitenleiste, die immer sichtbar ist, dazu eine volle
  Tabelle mit Aktuell/Min/Max/Ø und einem Verlaufsdiagramm. Während eines Tests
  wird alles als CSV aufgezeichnet. Jeder RAM-Fehler landet mit Zeitstempel in der
  Ereignisspalte, damit man sieht, welche Temperatur und Spannung in dem Moment anlagen.
- **SPD-Decoder** für DDR4 und DDR5 (JEDEC-Timings, XMP/EXPO-Erkennung,
  DDR4-XMP-2.0-Profile).
- **Protokoll, Screenshots (F12), Berichte (Strg+S)** als HTML, JSON und TXT.
  Rechnername und Seriennummern lassen sich ausblenden.
- **Hell/Dunkel**, Maus und Tastatur: F5 Start, Esc Stopp, Strg+1…7 Tabs.
- **Portabel**: eine einzige EXE, alle Daten in `RAMForge-data/` neben der EXE
  (Fallback `%LOCALAPPDATA%\RAMForge`). Statisch gelinkt, kein VC++-Redistributable nötig.
- **Updates**: Prüfung beim Start (abschaltbar). Installiert wird nur, wenn die
  Ed25519-Signatur des Manifests und die SHA-256-Prüfsumme der Datei stimmen.
  Keine Downgrades, nur HTTPS, nie während eines Tests. Die Dateien kommen von
  GitHub Releases, ersatzweise vom Spiegel auf ramforge.org. Das signierte Manifest
  liegt nur auf ramforge.org.
- **Neuigkeiten** vom Projekt in der App, als reiner Text.
- **Anonyme Statistik nur mit Einwilligung** (Standard: aus): zufällige ID, Version,
  Edition, grobe Windows-Version, höchstens einmal am Tag. Ein Widerruf löscht die Daten
  auf dem Server. Details: <https://ramforge.org/privacy>

## Boot-Variante und USB-Stick

`RAMForge-Boot-<version>.iso` startet ein minimales Debian-12-Live-System direkt in die
RAMForge-Oberfläche: kein Windows, keine Anmeldung, Maus und Tastatur, deutsches Layout.
Es schreibt nie auf interne Festplatten. Das Bootmenü bietet außerdem einen
Grafik-Sicherheitsmodus, Memtest86+ (nur ohne Secure Boot) und den Sprung in die
UEFI-Einstellungen.

**Stick einrichten mit `RAMForge-USB.exe`** (liegt neben `RAMForge.exe`, braucht
Administratorrechte):

1. ISO neben die EXE legen, Stick einstecken und in der Liste auswählen.
2. „Bootfähig einrichten“ wählen, den Datenträger prüfen und `LÖSCHEN` eintippen.
3. Der Stick wird mit MBR und FAT32 neu angelegt. Darauf landen die Boot-Dateien aus
   der ISO, die portable Windows-Version in `RAMForge\` und die Kennung `stick.json`.
   Danach wird die Kopie geprüft.
4. Später aktualisiert „Boot-Dateien aktualisieren“ den Stick ohne Formatieren.
   `RAMForge\RAMForge-data` mit Berichten und Logs bleibt erhalten.

Gesperrt sind: alles, was kein USB-Datenträger ist, der System- oder Startdatenträger,
der Datenträger, von dem das Programm läuft, und **jeder Stick mit einem Ordner
`RAMForge-Keys`**. Ein solcher Stick wird nie gelöscht.

Alternativ geht die ISO auch auf einen Ventoy-Stick (einfach kopieren) oder mit Rufus/dd
als Abbild. Dann landen Berichte aber nur im Arbeitsspeicher und gehen beim Ausschalten
verloren.

ISO selbst bauen (Docker Desktop):

```powershell
.\boot\debian\build.ps1
.\boot\debian\build.ps1 -Stage test
```

## Sensoren unter Windows

Ohne Treiber liefert Windows nur Auslastung, Takt, RAM-Belegung und ACPI-Temperaturen.
Für CPU-, DIMM- und Mainboard-Temperaturen, Spannungen, Lüfter, WHEA-Fehler und die
aktuellen Timings:

1. HWiNFO64 starten (Modus „Sensors“).
2. Einstellungen → **Shared Memory Support** aktivieren.
3. RAMForge erkennt das automatisch und zeigt alle Werte an.

Ein eigener Weg ohne HWiNFO (signierter PawnIO-Treiber) steht auf der Roadmap.

## Bauen

Voraussetzungen: Rust (stable, MSVC-Toolchain unter Windows).

```powershell
cargo test --workspace
cargo build --release
```

Die EXE liegt danach in `target\release\ramforge.exe` und kann direkt auf einen USB-Stick kopiert werden.

Kommandozeile:

| Schalter | Wirkung |
|---|---|
| `--version` | Version, Update-Manifest und Schlüssel-Fingerabdruck ausgeben |
| `--tab system\|test\|bench\|sensors\|spd\|log\|about` | mit diesem Tab starten |
| `--screenshot <datei.png>` | nach dem Start bzw. nach einem laufenden Test ein PNG speichern und beenden (Doku, CI) |
| `--start-test` | schnellen Selbsttest starten: 10 % des freien RAMs, ein künstlicher Bitfehler (prüft die ganze Kette bis zum Bericht) |
| `--start-test quick\|standard\|extreme` | dieses Profil ohne künstlichen Fehler auf 10 % des freien RAMs starten |
| `--start-bench` | Benchmark starten (128 MiB je Thread) |

Hardware-Probe für Entwickler:

```powershell
cargo run -p ramforge-hw --example probe --release
```

## Projektstruktur

```
crates/core          Testalgorithmen, Runner, Benchmark (plattformunabhängig)
crates/hw            SMBIOS, CPUID, SPD-Decoder, Sensorquellen, Sensor-Hub
crates/update        signierte Update-Prüfung, Selbstaustausch, Neuigkeiten, Opt-in-Statistik
apps/ramforge        RAMForge.exe: GUI (egui/eframe), Berichte, Protokoll
apps/ramforge-usb    RAMForge-USB.exe: USB-Stick vorbereiten/aktualisieren (Gerüst, schreibt noch nichts)
tools/ramforge-sign  Offline-Signierwerkzeug für Release-Manifeste
assets/logo          Logo und Icons
docs/                Architektur, Roadmap
```

`RAMForge.exe` und `RAMForge-USB.exe` werden zusammen ausgeliefert (gleicher Ordner bzw. gleiche ZIP-Datei).

### Release signieren (nur Maintainer)

Es gibt zwei Ed25519-Schlüssel. Eine Signatur gilt, wenn sie unter einem der beiden stimmt:

- **Release**: signiert jedes Update (Manifest und Dateien).
- **Notfall**: bleibt offline. Er wird nur gebraucht, wenn der Release-Schlüssel verloren
  geht oder gestohlen wird. Dann wird eine EXE mit neuem Release-Schlüssel veröffentlicht
  und mit dem Notfall-Schlüssel signiert.

Die privaten Schlüssel liegen nur verschlüsselt (Passphrase, scrypt + AES-256) auf dem
Rechner des Maintainers, nie im Repository und nie auf dem Server. Die Passphrase wird
nur interaktiv abgefragt, nie als Parameter oder Umgebungsvariable.

```powershell
cargo run -p ramforge-sign --release -- keygen --out $env:USERPROFILE\.ramforge\release.pem
cargo run -p ramforge-sign --release -- hash target\release\ramforge.exe
cargo run -p ramforge-sign --release -- sign $env:USERPROFILE\.ramforge\release.pem latest.json
cargo run -p ramforge-sign --release -- verify embedded latest.json
```

Fingerabdruck = SHA-256 über den 32-Byte-Schlüssel, Großbuchstaben-Hex in Vierergruppen
mit Leerzeichen. Beide Fingerabdrücke („Release“, „Notfall“) zeigen `ramforge.exe --version`,
der Tab „Über“ und die Website. Vor einem öffentlichen Release muss
`cargo test -p ramforge-update -- --ignored` grün sein. Das klappt erst, wenn beide echten
Schlüssel in `crates/update/src/key.rs` stehen.

Details: [docs/architecture.md](docs/architecture.md)

## Mitmachen

Beiträge sind willkommen: Code, Fehlerberichte, SPD-Dumps zum Testen des Decoders,
Übersetzungen. Siehe [CONTRIBUTING.md](CONTRIBUTING.md) und [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
Sicherheitslücken bitte vertraulich melden: [SECURITY.md](SECURITY.md).

## Rechtliches

- Lizenz: GPL-3.0-or-later, siehe [LICENSE](LICENSE).
  Copyright (C) 2026 Daniel Airich und RAMForge-Mitwirkende. RAMForge ist freie Software: Du kannst sie
  unter den Bedingungen der GNU General Public License, Version 3 oder (nach deiner Wahl)
  jeder späteren Version, weitergeben und/oder verändern.
- **Nutzung auf eigene Gefahr, ohne Gewähr:** [DISCLAIMER.md](DISCLAIMER.md) (Deutsch und Englisch).
- RAMForge ist eine Eigenentwicklung (Clean-Room). Es enthält keinen Code aus
  MemTest86, TestMem5 oder HWiNFO, und diese Programme wurden nicht dekompiliert.
  Alle genannten Marken gehören ihren Inhabern.
