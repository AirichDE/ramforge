#!/bin/bash
# Boots the RAMForge ISO in QEMU and takes screenshots (PPM) of the screen.
#
#   test-boot.sh bios      legacy BIOS, ISO as CD
#   test-boot.sh uefi      UEFI without Secure Boot, ISO as CD
#   test-boot.sh uefi-sb   UEFI with Secure Boot ON (Microsoft keys), ISO as CD
#   test-boot.sh usb-sb    Secure Boot ON, booted from a FAT32 USB stick image
#                          laid out exactly like RAMForge-USB.exe does it
#   test-boot.sh vhd-sb    Secure Boot ON, boots /out/test/stick.vhd: a virtual
#                          disk that RAMForge-USB.exe itself set up on Windows
#
# Output: /out/test/<mode>-<seconds>s.ppm, /out/test/<mode>-serial.log
set -euo pipefail
MODE="${1:-uefi-sb}"
VERSION="${RAMFORGE_VERSION:-0.3.0}"
ISO="${ISO:-/out/RAMForge-Boot-$VERSION.iso}"
SHOTS="${SHOTS:-45 150 300 420}"
T=/build/test-$MODE
OUT=/out/test
rm -rf "$T"
mkdir -p "$T" "$OUT"
test -f "$ISO" || { echo "ISO fehlt: $ISO"; exit 1; }

ACCEL=(-accel tcg)
[ -e /dev/kvm ] && ACCEL=(-accel kvm -cpu host)
# virtio-vga: KMS driver with PRIME support like real GPUs (QEMU's bochs "std"
# VGA lacks PRIME import, which wlroots/cage require).
COMMON=(-m 4096 -smp 4 -vga "${VGA:-virtio}" -display none -no-reboot
        -monitor "unix:$T/mon,server,nowait" -serial "file:$OUT/$MODE-serial.log")
FW=()
case "$MODE" in
    uefi)
        cp /usr/share/OVMF/OVMF_VARS_4M.fd "$T/vars.fd"
        FW=(-machine q35 -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd
            -drive if=pflash,format=raw,file="$T/vars.fd") ;;
    uefi-sb|usb-sb|vhd-sb)
        # Secure Boot enforced, Microsoft UEFI CA enrolled (what real PCs ship with).
        cp /usr/share/OVMF/OVMF_VARS_4M.ms.fd "$T/vars.fd"
        FW=(-machine q35,smm=on -global driver=cfi.pflash01,property=secure,value=on
            -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.secboot.fd
            -drive if=pflash,format=raw,unit=1,file="$T/vars.fd") ;;
    bios) FW=(-machine pc) ;;
    *) echo "unbekannter Modus $MODE"; exit 2 ;;
esac

MEDIA=(-cdrom "$ISO")
if [ "$MODE" = "usb-sb" ]; then
    # Stick layout of RAMForge-USB.exe: MBR, one FAT32 partition "RAMFORGE",
    # ISO content copied 1:1 plus the RAMForge folder.
    IMG="$T/stick.img"
    truncate -s 2G "$IMG"
    printf 'o\nn\np\n1\n2048\n\nt\nc\na\nw\n' | fdisk "$IMG" >/dev/null
    PART="$T/part.img"
    truncate -s $(( (2*1024*1024*1024) - 2048*512 )) "$PART"
    mkfs.vfat -F 32 -n RAMFORGE "$PART" >/dev/null
    mkdir -p "$T/iso"
    xorriso -osirrox on -indev "$ISO" -extract / "$T/iso" >/dev/null 2>&1
    chmod -R u+w "$T/iso"
    mkdir -p "$T/iso/RAMForge"
    printf '{"version":"%s","edition":"boot"}\n' "$VERSION" > "$T/iso/RAMForge/stick.json"
    MTOOLS_SKIP_CHECK=1 mcopy -s -i "$PART" "$T/iso/"* "$T/iso/".disk ::/ 2>/dev/null || MTOOLS_SKIP_CHECK=1 mcopy -s -i "$PART" "$T/iso/"* ::/
    dd if="$PART" of="$IMG" bs=1M seek=1 conv=notrunc status=none
    rm -f "$PART"
    # Attached as a plain disk: the firmware uses the same removable-media
    # path (EFI/boot/bootx64.efi); QEMU's emulated USB stack is slow and flaky.
    MEDIA=(-drive if=virtio,format=raw,file="$IMG")
fi

if [ "$MODE" = "vhd-sb" ]; then
    test -f "$OUT/stick.vhd" || { echo "fehlt: $OUT/stick.vhd"; exit 1; }
    cp "$OUT/stick.vhd" "$T/stick.vhd"
    MEDIA=(-drive if=virtio,format=vpc,file="$T/stick.vhd")
fi

qemu-system-x86_64 "${ACCEL[@]}" "${FW[@]}" "${COMMON[@]}" "${MEDIA[@]}" &
QPID=$!
start=$(date +%s)
for s in $SHOTS; do
    while [ $(( $(date +%s) - start )) -lt "$s" ]; do
        kill -0 $QPID 2>/dev/null || { echo "QEMU beendet nach $(( $(date +%s) - start )) s"; break 2; }
        sleep 2
    done
    python3 - "$T/mon" "$OUT/$MODE-${s}s.ppm" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX); s.settimeout(10); s.connect(sys.argv[1]); time.sleep(0.3)
try:
    s.recv(4096)
except OSError:
    pass
s.sendall(f"screendump {sys.argv[2]}\n".encode()); time.sleep(1.5); s.close()
PY
    echo "Screenshot nach ${s}s: $OUT/$MODE-${s}s.ppm"
done
kill $QPID 2>/dev/null || true
wait $QPID 2>/dev/null || true
echo "fertig: $MODE"
