#!/usr/bin/env python3
"""Brands live-build's boot loader templates for RAMForge.

usage: brand.py <config/bootloaders> <branding dir> <version>
GRUB (UEFI, Secure Boot) and ISOLINUX (legacy BIOS) get German menu entries,
the RAMForge splash and the author line "Daniel Airich".
"""
import shutil
import sys
from pathlib import Path

bl, branding, version = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
short = ".".join(version.split(".")[:2])


def patch(path: Path, pairs):
    s = path.read_text(encoding="utf-8")
    for a, b in pairs:
        if a not in s:
            raise SystemExit(f"{path}: Vorlage geändert, '{a[:40]}' fehlt")
        s = s.replace(a, b)
    path.write_text(s, encoding="utf-8")


# ---- GRUB (UEFI + BIOS via grub-pc templates) --------------------------------
g = bl / "grub-pc"
patch(g / "grub.cfg", [
    ("# Live boot\n@LINUX_LIVE@", f"""# RAMForge {version} – © 2026 Daniel Airich – https://ramforge.org
menuentry "RAMForge {short} starten" --hotkey=r {{
	linux	@KERNEL_LIVE@ @APPEND_LIVE@
	initrd	@INITRD_LIVE@
}}
menuentry "RAMForge {short} – Grafik-Sicherheitsmodus" --hotkey=g {{
	linux	@KERNEL_LIVE@ @APPEND_LIVE@ nomodeset
	initrd	@INITRD_LIVE@
}}"""),
    ("submenu 'Utilities...' --hotkey=u {", "submenu 'Werkzeuge …' --hotkey=w {"),
    ('menuentry "UEFI Firmware Settings" {', 'menuentry "UEFI-/BIOS-Einstellungen öffnen" {'),
])
patch(g / "config.cfg", [
    ("set default=0", "set default=0\nset timeout=5\nset timeout_style=menu"),
])
patch(g / "memtest.cfg", [
    ('"Memory Diagnostic Tool (@MEMTEST_VERSION@)"', '"Memtest86+ (@MEMTEST_VERSION@)"'),
])
# Last EFI memtest entry: unsigned, so it only starts with Secure Boot off.
s = (g / "memtest.cfg").read_text(encoding="utf-8")
s = s.replace('menuentry "Memtest86+ (@MEMTEST_VERSION@)" --hotkey=m {\n\t\tlinux @MEMTEST_DIR@/memtest.efi',
              'menuentry "Memtest86+ (@MEMTEST_VERSION@) – nur ohne Secure Boot" --hotkey=m {\n\t\tlinux @MEMTEST_DIR@/memtest.efi')
(g / "memtest.cfg").write_text(s, encoding="utf-8")
patch(g / "live-theme" / "theme.txt", [
    ('title-text: "Live Boot Menu with GRUB"', 'title-text: ""'),
    ("top = 52%", "top = 34%"),
    ('selected_item_color= "#ffffff"', 'selected_item_color= "#f26b2a"'),
    ('item_color = "#a8a8a8"', 'item_color = "#c8cdd6"'),
    ('fg_color = "#ffffff"', 'fg_color = "#f26b2a"'),
    ('text = "@KEYMAP_SHORT@"', 'text = "Enter: starten   ·   e: Startoptionen   ·   c: Kommandozeile"'),
    ('text = "@TIMEOUT_NOTIFICATION_LONG@"', 'text = "Start in %d s"'),
])
shutil.copy(branding / "splash.png", g / "splash.png")

# ---- ISOLINUX (legacy BIOS) ----------------------------------------------------
c = bl / "syslinux_common"
patch(c / "menu.cfg", [
    ("menu title Boot menu", f"menu title RAMForge {short} - Daniel Airich"),
    ("menu label ^Utilities", "menu label ^Werkzeuge"),
    ("menu title Utilities", "menu title Werkzeuge"),
    ("menu label ^Back..", "menu label ^Zurueck"),
])
patch(c / "live.cfg.in", [
    ("menu label ^Live system (@FLAVOUR@)", f"menu label ^RAMForge {short} starten"),
    ("menu label Live system (@FLAVOUR@ fail-safe mode)", f"menu label RAMForge {short} - Sicherheitsmodus"),
])
patch(c / "stdmenu.cfg", [
    ("menu tabmsg Press ENTER to boot or TAB to edit a menu entry", "menu tabmsg Enter = starten, Tab = Startoptionen"),
    ("#76a1d0ff", "#f26b2aff"),
])
patch(bl / "isolinux" / "isolinux.cfg", [("timeout 0", "timeout 50")])
(c / "splash.svg").unlink(missing_ok=True)
shutil.copy(branding / "splash-640.png", c / "splash.png")
shutil.copy(branding / "splash-800.png", c / "splash800x600.png")
print(f"Bootmenü gebrandet: RAMForge {version}")
