RAMForge – Lizenzen und Quellen / Licences and sources
======================================================

RAMForge (Programm, Logo, Boot-Konfiguration)
  © 2026 Daniel Airich und RAMForge-Mitwirkende
  GNU General Public License v3.0 oder später (RAMForge-GPL-3.0.txt)
  Quellcode: https://github.com/airichde/ramforge

Debian GNU/Linux 12 (Live-System auf diesem Medium)
  Enthält Pakete unter freien Lizenzen (u. a. GPL-2.0, GPL-3.0, LGPL, MIT, BSD)
  sowie nicht-freie Firmware (Debian-Bereich non-free-firmware, nur Variante "Voll").
  Die genaue Paketliste mit Versionen steht in /live/filesystem.packages.
  Quellcode jeder Paketversion: https://snapshot.debian.org/ bzw.
  https://sources.debian.org/ (Paketname + Version aus filesystem.packages).
  Copyright-Hinweise jedes Pakets: /usr/share/doc/<Paket>/copyright,
  vollständige Lizenztexte: /usr/share/common-licenses (beides im Live-System).

Memtest86+ (Booteintrag "Werkzeuge")
  GNU General Public License v2.0, https://www.memtest.org/ ,
  Quellcode: https://github.com/memtest86plus/memtest86plus
  Nicht signiert: startet nur mit ausgeschaltetem Secure Boot.

shim / GRUB / Linux-Kernel (Secure Boot)
  Von Debian signierte Pakete (shim-signed, grub-efi-amd64-signed,
  linux-image-amd64), unverändert übernommen.
