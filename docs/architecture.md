# Architektur

## Überblick

```
                ┌───────────────────────────────────────────────┐
                │ apps/ramforge  (GUI, egui/eframe)            │
                │  Tabs · Seitenleiste · Berichte · Protokoll   │
                └───────┬───────────────────────────┬───────────┘
                        │                           │
        ┌───────────────▼─────────────┐   ┌─────────▼──────────────────┐
        │ crates/core                 │   │ crates/hw                  │
        │  algo    Testverfahren      │   │  smbios   Board/BIOS/DIMMs │
        │  runner  Threads, Barrieren │   │  cpu      CPUID            │
        │  bench   Bandbreite/Latenz  │   │  spd      DDR4/DDR5-SPD    │
        │  mem     gesperrte Puffer   │   │  sensors  Quellen + Hub    │
        │  errors  Fehlersammlung     │   │           (+ CSV-Aufzeichn.)│
        └─────────────────────────────┘   └────────────────────────────┘
              plattformunabhängig           Windows-/Linux-Teil per cfg
```

Die **Windows-EXE** und die **Boot-Variante** (Linux-Live) bauen aus demselben Code.
Plattformabhängig sind nur `mem.rs` (VirtualAlloc/VirtualLock bzw. mmap/mlock),
`smbios::sys` (GetSystemFirmwareTable bzw. /sys/firmware/dmi) und die Sensorquellen.

## RAM-Test (`crates/core`)

- Jeder Worker-Thread reserviert seinen eigenen Puffer und sperrt ihn im RAM. Auf
  NUMA-Systemen landet der Speicher so neben dem Kern, der ihn testet.
- Alle Threads führen **denselben Test gleichzeitig** aus (Synchronisation über
  `Barrier`), damit der Speichercontroller voll ausgelastet ist.
- Ob abgebrochen wird, entscheidet ein Leader-Thread zwischen zwei Barrieren. So
  wartet nie ein Thread an einer Barriere, die die anderen nicht mehr erreichen.
- Alle Lese- und Schreibzugriffe sind `volatile`, damit der Compiler nichts wegoptimiert.
- Fehler gehen in einen `ErrorSink`: Zähler, die ersten 10 000 Fehler im Detail,
  ein Histogramm pro Datenbit und der Selbsttest-Schalter.
- **Selbsttest:** `fault_index` sorgt dafür, dass genau eine Prüfung einen
  geflippten Wert sieht. Das ist end-to-end prüfbar bis in Protokoll und Bericht.

### Testverfahren

| Test | Idee | findet vor allem |
|---|---|---|
| Eigene Adresse | Wort = eigene Adresse, danach invertiert | Adressleitungen, Aliasing |
| Moving Inversions 0/1, 8-Bit, Zufall | füllen, aufwärts prüfen und invertieren, abwärts prüfen und zurück | Stuck-at- und Kopplungsfehler |
| Zufallsfolge | reproduzierbare PRNG-Folge, prüfen, invertieren | datenabhängige Fehler |
| Zufallszugriff | Bijektion `i·k mod n`, verstreute Schreibzugriffe | Zeilenwechsel, Controller-Timing |
| Block verschieben | große Kopien, versetzt zurück | Burst-Transfers |
| Wandernde Bits | rotierende 1 bzw. 0 | Datenleitungen |
| Modulo 20 | Muster an jeder 20. Stelle, Störmuster dazwischen | Nachbareffekte, Cache-Einflüsse |
| Row Hammer (exp.) | Adresspaare mit `clflush` hämmern | schwache Zellen (nur zufällig ohne Adressmapping) |
| Bit Fade | füllen, warten, prüfen | Refresh- und Retention-Fehler |

## Sensoren (`crates/hw/src/sensors`)

`SensorSource::poll()` liefert `Reading`s (Gruppe, Gerät, Bezeichnung, Art, Wert,
Einheit). Der `SensorHub` fragt alle Quellen in einem eigenen Thread ab, führt
Min/Max/Ø und einen Verlauf (1800 Punkte) und schreibt auf Wunsch eine CSV-Datei.
Die Spalten werden beim Start der Aufzeichnung festgelegt, dazu kommt eine Spalte
`Ereignis`. RAM-Fehler werden dort eingetragen, sodass sie direkt neben Temperatur
und Spannung stehen.

| Quelle | Plattform | Inhalt | Rechte |
|---|---|---|---|
| `windows::BuiltinSource` | Windows | Last je Thread (NtQuerySystemInformation), effektiver Takt (PDH), RAM (GlobalMemoryStatusEx), ACPI-Thermalzonen (PDH) | keine |
| `hwinfo::HwinfoSource` | Windows | alle HWiNFO-Sensoren über dessen dokumentierte Shared-Memory-Schnittstelle | HWiNFO mit aktivem „Shared Memory Support“ |
| `linux::HwmonSource` | Linux | /sys/class/hwmon (coretemp, k10temp, nct6775, it87, spd5118, jc42, nvme …) | root im Live-System |
| `linux::ProcSource` | Linux | /proc/stat, cpufreq, /proc/meminfo | keine |

**Grenze unter Windows:** CPU-Paket-Temperatur (MSR), SuperIO-Spannungen und Lüfter,
SPD über SMBus und die aktuellen Timings aus dem Speichercontroller brauchen
Ring-0-Zugriff. RAMForge bringt dafür noch keinen Treiber mit. Geplant ist PawnIO
(signierter Treiber, GPL-2.0). Der früher übliche WinRing0 steht auf der
Microsoft-Sperrliste für anfällige Treiber und kommt nicht in Frage.

## SPD (`crates/hw/src/spd.rs`)

Bytes nach den JEDEC-Spezifikationen (DDR4: 21-C Annex L, DDR5: JESD400-5).
Bereits umgesetzt: Kapazität, Organisation, JEDEC-Timings (mit JEDEC-Rundung),
Hersteller (JEP106, Auswahl), Teilenummer, Datum, XMP/EXPO-Erkennung und
DDR4-XMP-2.0-Profile (experimentell). Datenquelle in der Boot-Variante sind die
Kernel-Treiber `ee1004` und `spd5118`, unter Windows eine Dump-Datei.

## Datenordner

`RAMForge-data/` neben der EXE (Fallback `%LOCALAPPDATA%\RAMForge`, Override über
`RAMFORGE_DATA`):

```
settings.json   Einstellungen, akzeptierte Disclaimer-Version
logs/           Sitzungsprotokolle
reports/        Berichte (.html, .json, .txt)
screenshots/    F12
sensors/        CSV-Aufzeichnungen
```

## Boot-Variante (`boot/debian`)

- Debian 12 Live, gebaut mit `live-build` in Docker (`boot/debian/build.ps1`).
- **Secure Boot:** `EFI/boot/bootx64.efi` ist der von Microsoft signierte Shim aus Debian, der den
  signierten GRUB und Kernel lädt. In QEMU mit OVMF und den Microsoft-Schlüsseln getestet
  (`build.ps1 -Stage test`).
- Ablauf: tty1-Autologin (root) → `ramforge-session` → `cage` (Wayland-Kiosk) → `ramforge`.
  Ohne GPU-Treiber wird per Software gezeichnet (`WLR_RENDERER=pixman`, llvmpipe). Wird RAMForge
  beendet, bietet die Konsole Neustart, Shell oder Ausschalten.
- Kernel-Module für Sensoren und SPD werden beim Start geladen. Sound-, WLAN-, Bluetooth- und
  Video-Treiber sind entfernt (`9100-trim.hook.chroot`).
- Daten: Auf einem FAT32-Stick von `RAMForge-USB.exe` landen Berichte in `RAMForge\RAMForge-data`
  (derselbe Ordner wie bei der portablen Windows-Version), sonst nur im RAM. Interne
  Datenträger werden nie eingehängt.
- Zwei Varianten: `RAMForge-Boot-<v>.iso` (Voll: GPU-Firmware, Vulkan, `ramforge-gpu`) und
  `RAMForge-Boot-Kompakt-<v>.iso` (ohne GPU-Teile).
- Bootmenü (GRUB und ISOLINUX): RAMForge starten, Grafik-Sicherheitsmodus (`nomodeset`),
  Werkzeuge mit Memtest86+ (EFI-Version nur ohne Secure Boot, da unsigniert) und UEFI-Einstellungen.

## USB-Werkzeug (`apps/ramforge-usb`)

Stand 0.1: Die Liste der Wechseldatenträger wird alle 2 Sekunden neu eingelesen, ein
eingesteckter Stick erscheint also von selbst. Ein RAMForge-Stick wird an
`RAMForge\stick.json` erkannt (`{"version": "x.y.z"}`). Erst wenn ein Stick ausgewählt
oder ein RAMForge-Stick eingesteckt wird, lädt das Werkzeug das signierte Manifest
(nie schon beim Programmstart). Daraus zeigt es pro Stick „aktuell“ oder
„veraltet: alt → neu“ und bietet bei Bedarf auch ein Update für sich selbst an
(gleiches Manifest, Asset `usb_tool`). Ventoy-Sticks werden erkannt und bekommen
einen Hinweis. Schreiben auf Laufwerke ist in 0.1 bewusst nicht eingebaut.

### Geplant

Ventoy-ähnliches Layout: EFI-Partition (FAT32) mit Bootloader und Kernel,
Datenpartition für Berichte und Updates. Updates tauschen nur die Dateien auf der
EFI-Partition aus (signiert, mit Prüfsumme, atomar über Umbenennen), ohne neu zu formatieren.

## Updates, Neuigkeiten, Statistik (`crates/update`)

Vertrauenskette beim Update, in dieser Reihenfolge:

1. `https://ramforge.org/update/latest.json` und `latest.json.sig` laden (nur HTTPS,
   Zertifikatsprüfung über den Windows-Zertifikatsspeicher).
2. Die Ed25519-Signatur über die **exakten Bytes** von `latest.json` prüfen. Gültig ist
   sie, wenn sie unter einem der zwei eingebauten Schlüssel stimmt (`RELEASE_KEY` oder
   `EMERGENCY_KEY` in `crates/update/src/key.rs`), und unter keinem anderen. Erst danach
   wird das JSON gelesen. Der Server allein ist nie vertrauenswürdig.
   Schlüsselwechsel: Eine neue EXE mit neuem `RELEASE_KEY` wird mit dem Notfall-Schlüssel
   signiert. Das Manifestformat ändert sich dabei nicht.
3. Nur echte Upgrades annehmen (keine Downgrades). Asset-URLs müssen HTTPS sein.
4. Datei neben die EXE laden (`RAMForge.exe.new`). Jedes Paket nennt im Manifest
   eine geordnete Liste `urls`: zuerst GitHub Releases, dann der Spiegel
   `https://ramforge.org/dl/`. Weiterleitungen (GitHub → `*.githubusercontent.com`)
   werden von Hand verfolgt, höchstens 5, nur zu `https://`. Scheitert eine Quelle
   (Netz, Zeitüberschreitung, HTTP-Fehler, mehr Bytes als `size`, falsche SHA-256),
   wird die Teildatei gelöscht und die nächste versucht. Scheitern alle, erscheint
   „Download fehlgeschlagen“ mit einem Link zur Download-Seite, und nichts wird geändert.
   Das Log nennt die benutzte Quelle („Quelle: GitHub“ bzw. „Spiegel ramforge.org“).
   Die Logik steckt in `crates/update/src/fetch.rs` hinter der Schnittstelle `Fetcher`
   und ist mit Unit-Tests ohne Netz abgedeckt.
5. `RAMForge.exe` → `RAMForge.exe.old`, `.new` → `RAMForge.exe`, neue Version starten,
   alte beenden. Der nächste Start löscht `.old`. Während eines Tests wird nie aktualisiert.

Dieselbe Prüfung nutzen `RAMForge-USB.exe` (Asset `usb_tool`) und später das
Stick-Update (Asset `boot_iso`). Der gemeinsame Dialog liegt in
`crates/update/src/ui.rs` (Feature `ui`).

**Neuigkeiten** (`/api/v1/news?lang=de&version=…`) werden als reiner Text angezeigt,
nie als HTML. Links nur mit HTTPS. Die höchste gesehene ID steht in `settings.json`.

**Statistik** (`/api/v1/ping`) nur nach ausdrücklicher Einwilligung im Startdialog,
Standard aus. Gesendet werden eine lokal erzeugte UUIDv4 (nicht aus der Hardware
abgeleitet), die Version, die Edition und eine grobe Betriebssystem-Angabe, höchstens
einmal am Tag. Beim Widerruf wird `/api/v1/forget` aufgerufen und die lokale ID gelöscht.
Hardwaredaten, Seriennummern, Ergebnisse und Logs verlassen den Rechner nie automatisch.
