# Haftungsausschluss / Disclaimer

**Version 1 · Stand: 25.11.2026**

> Deutsche Fassung zuerst, English version below. Gibt es Abweichungen zwischen
> der deutschen und der englischen Fassung, gilt die deutsche.
> Website: https://ramforge.org/disclaimer

---

## Deutsch

### 1. Nutzung auf eigene Gefahr

RAMForge belastet Arbeitsspeicher, Prozessor und Speichercontroller bewusst bis an die Grenze. Dabei kann unter anderem Folgendes passieren:

- Temperaturen und Leistungsaufnahme von CPU, RAM, Spannungswandlern und Netzteil steigen deutlich.
- Instabile, übertaktete, untertaktete oder fehlerhaft gekühlte Systeme können abstürzen, einfrieren, neu starten oder sich abschalten.
- Ungespeicherte Daten in anderen Programmen können dabei verloren gehen. Selten können auch Dateisysteme beschädigt werden.
- Bereits vorgeschädigte Hardware kann unter Last endgültig ausfallen.

**Vor jedem Test:** Daten sichern, alle anderen Programme schließen, für ausreichende Kühlung sorgen, Notebooks am Netzteil betreiben und den Rechner während des Tests nicht unbeaufsichtigt lassen. Bei Übertaktung oder manuell veränderten Spannungen trägst du das Risiko allein.

### 2. USB-Werkzeug und Boot-System

Das USB-Werkzeug (RAMForge-USB.exe) schreibt auf den ausgewählten Datenträger. **Bei der Ersteinrichtung werden alle Daten auf diesem Datenträger unwiderruflich gelöscht.** Prüfe vor dem Bestätigen sorgfältig, dass du den richtigen Datenträger ausgewählt hast. Wählst du versehentlich eine falsche Festplatte oder einen falschen Stick, sind die Daten darauf verloren.

Das Boot-System startet ohne installiertes Betriebssystem. Es ist so gebaut, dass es auf interne Festplatten und SSDs nicht schreibt. Trotzdem kann ein Absturz während eines Tests nicht ausgeschlossen werden. Änderungen an UEFI/BIOS-Einstellungen (zum Beispiel Secure Boot, Bootreihenfolge) nimmst du auf eigene Verantwortung vor.

### 3. Keine Gewähr für Testergebnisse und Messwerte

- Ein bestandener Test belegt nicht, dass der Arbeitsspeicher fehlerfrei ist. Manche Fehler treten nur selten, nur bei bestimmten Temperaturen oder nur unter anderer Last auf.
- Ein gemeldeter Fehler muss nicht am RAM-Modul liegen. Ursache können auch Prozessor, Speichercontroller, Mainboard, BIOS-Einstellungen, Netzteil, Übertaktung oder Software sein.
- Benchmark-Ergebnisse hängen von vielen Faktoren ab und sind nur eingeschränkt vergleichbar.
- Sensor-, SPD- und Systemwerte stammen von der Hardware, von Firmware-Tabellen, von Treibern oder von Drittsoftware (zum Beispiel über die Shared-Memory-Schnittstelle von HWiNFO). Sie können ungenau, falsch zugeordnet oder veraltet sein.

RAMForge ist kein zertifiziertes Diagnose- oder Prüfwerkzeug. Stütze Kauf-, Reklamations-, Garantie- oder Sicherheitsentscheidungen nicht allein auf die Ergebnisse.

### 4. Gewährleistung und Haftung

RAMForge ist freie Software und wird **unentgeltlich** unter der GNU General Public License v3.0 überlassen. Nach den Abschnitten 15 und 16 der Lizenz wird die Software „wie sie ist“ bereitgestellt, ohne Gewährleistung für Marktreife, Eignung für einen bestimmten Zweck oder Fehlerfreiheit, soweit das anwendbare Recht dies zulässt.

Für Nutzer, auf die deutsches Recht anwendbar ist, gilt: Da die Software unentgeltlich überlassen wird, haften die Urheber und Mitwirkenden nur für **Vorsatz und grobe Fahrlässigkeit** sowie für arglistig verschwiegene Mängel (entsprechend §§ 521, 523, 524, 599, 600 BGB). Unberührt bleibt die Haftung für Schäden aus der Verletzung des Lebens, des Körpers oder der Gesundheit und die Haftung nach dem Produkthaftungsgesetz, soweit diese zwingend ist.

Im Übrigen ist jede Haftung für Schäden an Hardware, Datenverlust, entgangenen Gewinn, Betriebsunterbrechung oder sonstige mittelbare oder unmittelbare Schäden aus der Nutzung oder Nichtnutzbarkeit der Software ausgeschlossen, soweit gesetzlich zulässig.

### 5. Marken und Unabhängigkeit

RAMForge ist ein unabhängiges Projekt und steht in keiner Verbindung zu den Herstellern anderer Speichertest- oder Diagnoseprogramme oder von Hardware. Genannte Namen wie MemTest86 (PassMark Software), Memtest86+, HWiNFO, TestMem5, Intel, AMD, XMP oder EXPO sind Marken oder Bezeichnungen ihrer jeweiligen Inhaber und werden nur zur Beschreibung von Kompatibilität oder Funktionen genannt. RAMForge enthält keinen Code aus proprietären Programmen. Alle Testverfahren sind eigenständig auf Grundlage öffentlich dokumentierter Methoden umgesetzt.

### 6. Sponsor

Die Server-Infrastruktur stellt Nerdscave Consulting (https://nerdscave-consulting.com/) freundlicherweise kostenlos zur Verfügung. Der Sponsor ist an Entwicklung, Inhalten und Betrieb von RAMForge nicht beteiligt und übernimmt keine Verantwortung oder Haftung für die Software oder die Website.

---

## English

### 1. Use at your own risk

RAMForge deliberately pushes memory, processor and memory controller to their limits. Among other things, the following may happen:

- Temperatures and power draw of CPU, RAM, voltage regulators and power supply rise considerably.
- Unstable, overclocked, underclocked or poorly cooled systems may crash, freeze, reboot or shut down.
- Unsaved data in other programs may be lost. In rare cases file systems may be damaged.
- Hardware that is already damaged may fail completely under load.

**Before every test:** back up your data, close all other programs, make sure cooling is adequate, run notebooks on mains power, and don't leave the computer unattended during the test. If you overclock or change voltages manually, you bear the risk alone.

### 2. USB tool and boot system

The USB tool (RAMForge-USB.exe) writes to the selected drive. **The initial setup irreversibly erases all data on that drive.** Before confirming, check carefully that you selected the correct drive. If you pick the wrong disk or stick by mistake, its data is lost.

The boot system starts without an installed operating system. It is designed not to write to internal hard disks or SSDs. Even so, a crash during a test cannot be ruled out. Any changes to UEFI/BIOS settings (for example Secure Boot or boot order) are your own responsibility.

### 3. No guarantee for test results and readings

- Passing a test does not prove that the memory is free of defects. Some faults occur only rarely, only at certain temperatures or only under different load.
- A reported error is not necessarily caused by the memory module. It may also come from the processor, memory controller, mainboard, BIOS settings, power supply, overclocking or software.
- Benchmark results depend on many factors and are only comparable to a limited extent.
- Sensor, SPD and system values come from the hardware, firmware tables, drivers or third-party software (for example HWiNFO's shared memory interface). They may be inaccurate, mislabelled or outdated.

RAMForge is not a certified diagnostic or testing tool. Don't base purchase, return, warranty or safety decisions on its results alone.

### 4. Warranty and liability

RAMForge is free software provided **free of charge** under the GNU General Public License v3.0. Under sections 15 and 16 of the license, the software is provided "as is", without warranty of merchantability, fitness for a particular purpose or freedom from defects, to the extent permitted by applicable law.

For users subject to German law: since the software is provided free of charge, the authors and contributors are liable only for **intent and gross negligence** and for fraudulently concealed defects (in accordance with §§ 521, 523, 524, 599, 600 BGB). Liability for injury to life, body or health and mandatory liability under the German Product Liability Act remain unaffected.

Otherwise, to the extent permitted by law, all liability is excluded for damage to hardware, loss of data, lost profits, business interruption or any other direct or indirect damage arising from the use of, or inability to use, the software.

### 5. Trademarks and independence

RAMForge is an independent project with no affiliation to the makers of other memory test or diagnostic programs or of hardware. Names such as MemTest86 (PassMark Software), Memtest86+, HWiNFO, TestMem5, Intel, AMD, XMP or EXPO are trademarks or designations of their respective owners and are used only to describe compatibility or features. RAMForge contains no code from proprietary programs. All test methods are independent implementations of publicly documented techniques.

### 6. Sponsor

The server infrastructure is kindly provided free of charge by Nerdscave Consulting (https://nerdscave-consulting.com/). The sponsor is not involved in the development, content or operation of RAMForge and accepts no responsibility or liability for the software or the website.
