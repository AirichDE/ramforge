#!/bin/bash
# Test copy of a boot ISO for the X fallback experiments:
#   - default GRUB entry gets "nomodeset ramforge.debug" (no DRM driver for the
#     PCI VGA, efifb only = a real PC whose GPU is newer than the kernel)
#   - debug/run.sh is added as /ramforge-debug/run.sh
# usage (in the builder container): mk-debug-iso.sh /out/<iso> [/out/test-debug.iso]
set -euo pipefail
SRC=$1
DST=${2:-/out/test-debug.iso}
xorriso -osirrox on -indev "$SRC" -extract /boot/grub/grub.cfg /tmp/g.cfg >/dev/null 2>&1
chmod u+w /tmp/g.cfg
EXTRA="nomodeset"; [ "${NODEBUG:-}" = 1 ] || EXTRA="nomodeset ramforge.debug"
awk -v x="$EXTRA" 'BEGIN{d=0} /^	linux/ && !d {print $0" "x; d=1; next} {print}' /tmp/g.cfg > /tmp/g2.cfg
grep -m1 nomodeset /tmp/g2.cfg >/dev/null || { echo "grub.cfg nicht angepasst"; exit 1; }
rm -f "$DST"
xorriso -indev "$SRC" -outdev "$DST" -boot_image any replay \
    -update /tmp/g2.cfg /boot/grub/grub.cfg \
    -map /boot-src/debug/run.sh /ramforge-debug/run.sh >/dev/null 2>&1
ls -la "$DST"
