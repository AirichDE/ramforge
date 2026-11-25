# Sicherheit / Security

## Unterstützte Versionen

Sicherheitsupdates gibt es nur für die jeweils neueste Version.

## Sicherheitslücke melden

Bitte **kein öffentliches Issue** anlegen. Melde Lücken vertraulich:

- E-Mail: `report@ramforge.org` (Betreff „Security“)
- oder über „Report a vulnerability“ (GitHub Private Vulnerability Reporting) auf
  <https://github.com/airichde/ramforge>

Bitte beschreibe Version, betroffene Komponente (EXE, Boot-Medium, USB-Werkzeug,
Update-Server), Schritte zum Nachstellen und mögliche Auswirkungen. Wir bestätigen
den Eingang innerhalb von 7 Tagen und stimmen den Veröffentlichungszeitpunkt mit dir ab.

## Update-Schlüssel

Updates sind mit Ed25519 signiert. Die App vertraut genau zwei öffentlichen Schlüsseln:
**Release** (signiert jedes Update) und **Notfall** (offline, nur für den Schlüsselwechsel,
falls der Release-Schlüssel verloren geht oder gestohlen wird). Die privaten Schlüssel
liegen nur passwortverschlüsselt beim Maintainer.

Die Fingerabdrücke (SHA-256 über den Schlüssel, Vierergruppen mit Leerzeichen) stehen auf
<https://ramforge.org>, im Tab „Über“ und in der Ausgabe von `ramforge.exe --version`.
Weichen die Angaben voneinander ab: nichts installieren und bitte melden.

| Schlüssel | Fingerabdruck (SHA-256) |
|---|---|
| Release | `79D1 AF24 BEAD 4207 50B6 D310 02F0 B850 677A D831 D426 A841 FBFD CF06 1868 87E1` |
| Notfall | `8568 B6C3 10CA 2122 F398 9AB6 E441 E77E B6AE 3BB3 7ED3 780F 021A 6308 E82E 288A` |

## Besonders relevante Bereiche

- **USB-Werkzeug**: formatiert Laufwerke. Fehler bei der Laufwerksauswahl können Daten zerstören.
- **Update-Funktion**: Updates müssen signiert und geprüft sein, bevor sie geschrieben werden.
- **Treiber** (geplant, PawnIO): Kernelzugriff darf nicht für andere Programme missbrauchbar sein.

## English

Please do not open public issues for vulnerabilities. Report them privately to
`report@ramforge.org` (subject "Security") or via GitHub Private Vulnerability
Reporting on https://github.com/airichde/ramforge. We acknowledge reports
within 7 days.
