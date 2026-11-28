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
# Default language of the boot edition is English (the PC cannot tell us the
# user's language); the German entries override the kernel locale. live-config
# and ramforge-prepare both take the last locales=/keyboard-layouts= value.
DE = "locales=de_DE.UTF-8 keyboard-layouts=de"


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
menuentry "Start RAMForge {short} (English)" --hotkey=r {{
	linux	@KERNEL_LIVE@ @APPEND_LIVE@
	initrd	@INITRD_LIVE@
}}
menuentry "RAMForge {short} starten (Deutsch)" --hotkey=d {{
	linux	@KERNEL_LIVE@ @APPEND_LIVE@ {DE}
	initrd	@INITRD_LIVE@
}}
menuentry "Safe graphics mode (English)" --hotkey=g {{
	linux	@KERNEL_LIVE@ @APPEND_LIVE@ nomodeset
	initrd	@INITRD_LIVE@
}}
menuentry "Grafik-Sicherheitsmodus (Deutsch)" --hotkey=s {{
	linux	@KERNEL_LIVE@ @APPEND_LIVE@ {DE} nomodeset
	initrd	@INITRD_LIVE@
}}"""),
    ("submenu 'Utilities...' --hotkey=u {", "submenu 'Tools / Werkzeuge …' --hotkey=t {"),
    ('menuentry "UEFI Firmware Settings" {', 'menuentry "UEFI/BIOS settings / Einstellungen" {'),
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
              'menuentry "Memtest86+ (@MEMTEST_VERSION@) – Secure Boot off only / nur ohne Secure Boot" --hotkey=m {\n\t\tlinux @MEMTEST_DIR@/memtest.efi')
(g / "memtest.cfg").write_text(s, encoding="utf-8")
patch(g / "live-theme" / "theme.txt", [
    ('title-text: "Live Boot Menu with GRUB"', 'title-text: ""'),
    ("top = 52%", "top = 34%"),
    ('selected_item_color= "#ffffff"', 'selected_item_color= "#f26b2a"'),
    ('item_color = "#a8a8a8"', 'item_color = "#c8cdd6"'),
    ('fg_color = "#ffffff"', 'fg_color = "#f26b2a"'),
    ('text = "@KEYMAP_SHORT@"', 'text = "Enter: start   ·   e: boot options   ·   c: command line"'),
    ('text = "@TIMEOUT_NOTIFICATION_LONG@"', 'text = "Start in %d s"'),
])
shutil.copy(branding / "splash.png", g / "splash.png")

# ---- ISOLINUX (legacy BIOS) ----------------------------------------------------
c = bl / "syslinux_common"
patch(c / "menu.cfg", [
    ("menu title Boot menu", f"menu title RAMForge {short} - Daniel Airich"),
    ("menu label ^Utilities", "menu label ^Tools / Werkzeuge"),
    ("menu title Utilities", "menu title Tools / Werkzeuge"),
    ("menu label ^Back..", "menu label ^Back / Zurueck"),
])
patch(c / "live.cfg.in", [
    ("menu label ^Live system (@FLAVOUR@)", f"menu label ^Start RAMForge {short} (English)"),
    ("\tappend @APPEND_LIVE@\n\nlabel live-@FLAVOUR@-failsafe",
     f"\tappend @APPEND_LIVE@\n\nlabel live-@FLAVOUR@-de\n\tmenu label RAMForge {short} starten (^Deutsch)\n"
     f"\tlinux @LINUX@\n\tinitrd @INITRD@\n\tappend @APPEND_LIVE@ {DE}\n\nlabel live-@FLAVOUR@-failsafe"),
    ("menu label Live system (@FLAVOUR@ fail-safe mode)", f"menu label RAMForge {short} - Safe mode / Sicherheitsmodus"),
])
patch(c / "stdmenu.cfg", [
    ("menu tabmsg Press ENTER to boot or TAB to edit a menu entry", "menu tabmsg Enter = start, Tab = boot options"),
    ("#76a1d0ff", "#f26b2aff"),
])
patch(bl / "isolinux" / "isolinux.cfg", [("timeout 0", "timeout 50")])
(c / "splash.svg").unlink(missing_ok=True)
shutil.copy(branding / "splash-640.png", c / "splash.png")
shutil.copy(branding / "splash-800.png", c / "splash800x600.png")
print(f"Bootmenü gebrandet: RAMForge {version}")
