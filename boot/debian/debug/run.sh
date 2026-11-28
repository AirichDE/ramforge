#!/bin/bash
# X fallback experiments, run as root by ramforge-prepare when the kernel
# command line has "ramforge.debug" and this file is on the boot medium as
# ramforge-debug/run.sh. Tries several Xorg configs on the firmware
# framebuffer and prints which one gets a screen. Nothing is written to disk.
set -u
# Detach: ramforge-prepare runs under a systemd start timeout. Run once, in the
# background, after the normal session had its go; output to serial console.
if [ -z "${RF_BG:-}" ]; then
    mkdir /run/xexp.lock 2>/dev/null || exit 0
    RF_BG=1 setsid bash "$0" </dev/null >>/dev/ttyS0 2>&1 &
    exit 0
fi
sleep 100
X=/etc/X11
BUSID=$(sed -n 's/.*BusID *"\(.*\)".*/\1/p' $X/ramforge-fbdev-pci.conf 2>/dev/null)
echo "== X-Experimente  fb: $(tr '\n' ' ' </proc/fb)  BusID: ${BUSID:-?}"

base() { # $1 = extra Device lines, $2 = extra ServerFlags lines
cat <<EOF
Section "Device"
    Identifier "FB"
    Driver     "fbdev"
    Option     "fbdev" "/dev/fb0"
$1
EndSection
Section "Monitor"
    Identifier "Mon"
EndSection
Section "Screen"
    Identifier "Screen0"
    Device     "FB"
    Monitor    "Mon"
EndSection
Section "ServerLayout"
    Identifier "Layout"
    Screen     "Screen0"
EndSection
Section "ServerFlags"
    Option "AutoAddGPU" "false"
$2
EndSection
EOF
}
base "" ""                                         > $X/exp-a.conf  # full layout
base '    BusID "fb:0"' ""                         > $X/exp-b.conf  # non-PCI BusID: skips PCI probe
base '    BusID "ramforge"' ""                     > $X/exp-c.conf

try() { # name, extra Xorg args
    local n=$1; shift
    timeout 25 xinit /bin/sleep 4 -- /usr/lib/xorg/Xorg :7 vt7 -nolisten tcp \
        -logfile /run/xexp-$n.log "$@" >/dev/null 2>&1
    local rc=$?
    if grep -q "no screens found\|Fatal server error" /run/xexp-$n.log 2>/dev/null; then
        echo "-- $n: FEHLER (rc $rc)"
        sed -n '/LoadModule: "fbdev"/,$p' /run/xexp-$n.log | grep -vE "font path|Entry deleted|ABI class|compiled for|\(EE\) *$|Please also|wiki|^\s*$" \
            | head -n "${FULL:-6}" | sed 's/^/     /'
        echo "     ... Ende des Logs:"
        grep -vE "font path|Entry deleted|^\s*$" /run/xexp-$n.log | tail -n 14 | sed 's/^/     /'
    else
        echo "-- $n: OK? (rc $rc, rc 0 = X lief und der Client lief durch)"
        grep -E "FBDEV\(0\)|\(EE\)" /run/xexp-$n.log 2>/dev/null | grep -vE "\(EE\) *$" | head -n 6 | sed 's/^/     /'
    fi
}
try b -config exp-b.conf
# Xorg's fbdevhw refuses /dev/fb0 when /sys/class/graphics/fb0 points below
# devices/pci (efifb on a PCI card since kernel 6.x). Show it a copy of that
# class directory where fb0 looks like a platform device.
D=/sys/class/graphics
echo "   fb0 -> $(readlink $D/fb0)"
mkdir -p /run/rf-graphics
for e in "$D"/*; do ln -sfn "$(readlink "$e")" "/run/rf-graphics/$(basename "$e")"; done
ln -sfn ../../devices/platform/efi-framebuffer.0/graphics/fb0 /run/rf-graphics/fb0
mount --bind /run/rf-graphics "$D" && echo "   sysfs-Umgehung aktiv: fb0 -> $(readlink $D/fb0)"
F=/sys/devices/platform/efi-framebuffer.0/graphics/fb0
[ -d "$F" ] || F=$(readlink -f /run/rf-graphics/../../sys/class/graphics/fb0 2>/dev/null)
F=$(find /sys/devices -path '*graphics/fb0' -type d 2>/dev/null | head -n 1)
for k in virtual_size bits_per_pixel stride modes name; do echo "   fb0 $k: $(tr '\n' ' ' < "$F/$k" 2>/dev/null)"; done
command -v fbset >/dev/null && fbset -i -fb /dev/fb0 | sed 's/^/   /'
VS=$(cat "$F/virtual_size" 2>/dev/null)   # e.g. 800,600
MODE=$(sed -n 's/^.:\([0-9]*x[0-9]*\).*/\1/p' "$F/modes" 2>/dev/null | head -n 1)
base '    BusID "fb:0"' "" | sed "s|^    Monitor    \"Mon\"|&\n    SubSection \"Display\"\n        Virtual ${VS/,/ }\n    EndSubSection|" > $X/exp-v.conf
grep -A2 SubSection $X/exp-v.conf | sed 's/^/   /'
FULL=4 try b2 -config exp-b.conf
FULL=4 try v -config exp-v.conf
umount "$D"
echo "== Ende X-Experimente"
rm -f $X/exp-?.conf
