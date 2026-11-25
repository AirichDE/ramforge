# Mitmachen bei RAMForge / Contributing

Danke, dass du helfen willst! Jede Hilfe zählt: Code, Fehlerberichte, Tests auf
deiner Hardware, SPD-Dumps, Doku, Übersetzungen.
*English contributions are equally welcome. Issues and pull requests may be in
German or English.*

## Fehler melden

Bitte ein Issue anlegen mit:
- RAMForge-Version, Windows-Version oder Boot-Variante
- CPU, Mainboard, RAM-Module (der Tab **System** oder ein Bericht hilft)
- was passiert ist und was du erwartet hättest
- den Bericht (Strg+S, die `.txt`- oder `.json`-Datei). Rechnername und
  Seriennummern sind dort standardmäßig ausgeblendet.

**Speicherfehler, die RAMForge findet, sind meist echte Hardware- oder
Einstellungsprobleme**, keine Programmfehler. Bitte zuerst mit Standardeinstellungen
im BIOS (ohne XMP/EXPO) gegentesten.

## SPD-Dumps

Der DDR4/DDR5-Decoder braucht echte Dumps zum Prüfen, besonders für XMP 3.0 und EXPO.
Wenn du einen SPD-Dump (`.bin`) und die Herstellerangaben (Datenblatt, Aufkleber) hast,
hänge beides an ein Issue. SPD-Dumps enthalten eine Modul-Seriennummer. Wer sie nicht
teilen will, überschreibt die Bytes 325–328 (DDR4) bzw. 517–520 (DDR5) mit Nullen.

## Code beitragen

1. Repository forken, Branch anlegen (`fix/…`, `feat/…`).
2. `cargo fmt`, `cargo clippy --workspace` und `cargo test --workspace` müssen sauber durchlaufen.
3. Neue Testalgorithmen brauchen einen Unit-Test in `crates/core/src/algo.rs`, der
   sowohl „gesunder Speicher, 0 Fehler“ als auch „eingespeister Fehler wird gefunden“ prüft.
4. Pull Request mit kurzer Beschreibung: was, warum, wie getestet.

### Regeln für Code

- **Clean-Room:** Kein Code, keine Tabellen und keine Strukturen aus proprietärer
  Software (MemTest86, TestMem5, HWiNFO, AIDA64 usw.), auch nicht aus Dekompilaten.
  Verwende öffentliche Spezifikationen (JEDEC, DMTF SMBIOS, Datenblätter),
  Fachliteratur oder Code unter einer GPL-3.0-kompatiblen Lizenz und nenne die Quelle.
- Alles, was direkt auf Hardware zugreift, hinter einer klaren Schnittstelle
  (`SensorSource`, `TestBuffer`), damit Windows und Boot-Variante denselben Kern nutzen.
- Kein Netzwerkzugriff ohne Opt-in der Nutzerin bzw. des Nutzers.
- Oberflächentexte vorerst auf Deutsch. Mehrsprachigkeit steht auf der Roadmap.

## Lizenz deiner Beiträge

Mit einem Pull Request stimmst du zu, dass dein Beitrag unter der
GPL-3.0-or-later veröffentlicht wird. Es gibt kein CLA. Bitte bestätige mit
`git commit -s` (Developer Certificate of Origin, <https://developercertificate.org>),
dass du das Recht hast, den Code beizutragen.

## Verhalten

Es gilt der [Verhaltenskodex](CODE_OF_CONDUCT.md).
