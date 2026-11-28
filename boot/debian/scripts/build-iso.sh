#!/bin/bash
# Builds RAMForge-Boot-<version>.iso (Debian 12 live, Secure Boot capable).
# Needs: docker run --privileged, /out/ramforge-linux from build-app.sh.
set -euo pipefail
VERSION="${RAMFORGE_VERSION:-0.3.0}"
# full    = all GPU firmware, Vulkan, GPU benchmark module (~ largest)
# compact = RAM test, sensors, CPU/RAM benchmarks; software graphics only
# ram     = only the RAM test (plus sensors/SPD): no benchmarks, no network,
#           no GPU/network/sound drivers (smallest)
EDITION="${RAMFORGE_EDITION:-full}"
W=/build/live-work-$EDITION
case "$EDITION" in
    full) NAME="RAMForge-Boot-$VERSION.iso" ;;
    compact) NAME="RAMForge-Boot-Kompakt-$VERSION.iso" ;;
    ram) NAME="RAMForge-Boot-RAM-$VERSION.iso" ;;
    *) echo "unbekannte Edition $EDITION"; exit 2 ;;
esac
# Kernel: "new" (default since 0.3.1) = bookworm-backports 6.12, Debian-signed
# for Secure Boot, with current firmware and Mesa: needed for GPUs newer than
# 6.1, e.g. NVIDIA RTX 40 (nouveau + GSP firmware). "stable" = Debian 12's 6.1,
# only for comparison builds (file name gets "-Kernel61").
KERNEL="${RAMFORGE_KERNEL:-new}"
if [ "$KERNEL" = "stable" ]; then
    NAME="${NAME%-$VERSION.iso}-Kernel61-$VERSION.iso"
    W="$W-k61"
fi
test -x /out/ramforge-linux || { echo "Fehlt: /out/ramforge-linux (erst build-app.sh)"; exit 1; }
rm -rf "$W"
mkdir -p "$W"
cp -r --no-preserve=mode,ownership /boot-src/live/. "$W/"
cd "$W"
# Files from a Windows bind mount arrive as 0777: normalise before they reach the image.
find config auto -type d -exec chmod 0755 {} +
find config auto -type f -exec chmod 0644 {} +
chmod 0755 auto/config config/hooks/normal/*.hook.chroot config/includes.chroot/usr/local/bin/*
# Package cache survives between builds (second edition builds much faster).
mkdir -p /build/lb-cache
ln -s /build/lb-cache cache
if [ "$EDITION" = "full" ]; then
    install -D -m0755 /out/ramforge-gpu-linux config/includes.chroot/usr/local/bin/ramforge-gpu
    printf '# GPU benchmark (Vulkan)\nmesa-vulkan-drivers\nlibvulkan1\n' >> config/package-lists/ramforge.list.chroot
else
    sed -i -E '/^(firmware-amd-graphics|firmware-misc-nonfree|amd64-microcode|intel-microcode)$/d' config/package-lists/ramforge.list.chroot
fi
if [ "$EDITION" = "ram" ]; then
    # No network (no update check or news in this variant), no USB/PCI listing tools.
    sed -i -E '/^(ca-certificates|systemd-resolved|usbutils)$/d' config/package-lists/ramforge.list.chroot
fi
if [ "$KERNEL" = "new" ]; then
    mkdir -p config/archives
    echo "deb http://deb.debian.org/debian bookworm-backports main contrib non-free-firmware"         > config/archives/backports.list.chroot
    # Only kernel, firmware and the graphics stack come from backports.
    cat > config/archives/backports.pref.chroot <<'PREF'
Package: linux-image-* linux-base firmware-* *mesa* libdrm* libgl1* libglx* libegl* libgbm* libglapi* libvulkan* libllvm* libxatracker* libosmesa* libz3-* libclc-*
Pin: release n=bookworm-backports
Pin-Priority: 990
PREF
    if [ "$EDITION" != "ram" ]; then
        # Split firmware packages exist only in backports (NVIDIA GSP for RTX 40, Intel GuC/HuC).
        printf '# GPU firmware (backports split packages)
firmware-nvidia-graphics
firmware-intel-graphics
' >> config/package-lists/ramforge.list.chroot
    fi
fi
echo "$KERNEL" > config/includes.chroot/etc/ramforge-kernel
echo "$EDITION" > config/includes.chroot/etc/ramforge-edition
install -D -m0755 /out/ramforge-linux config/includes.chroot/usr/local/bin/ramforge
install -D -m0644 /src/assets/logo/out/ramforge-icon-256.png config/includes.chroot/usr/local/share/ramforge/logo.png
install -D -m0644 /src/DISCLAIMER.md config/includes.chroot/usr/local/share/ramforge/DISCLAIMER.md
install -D -m0644 /src/LICENSE config/includes.chroot/usr/local/share/ramforge/LICENSE
# Licence and source notes on the boot medium itself.
install -D -m0644 /src/LICENSE config/includes.binary/LICENSES/RAMForge-GPL-3.0.txt
install -D -m0644 /boot-src/licenses/README.txt config/includes.binary/LICENSES/README.txt
mkdir -p config/bootloaders
cp -a /usr/share/live/build/bootloaders/grub-pc /usr/share/live/build/bootloaders/syslinux_common \
      /usr/share/live/build/bootloaders/isolinux config/bootloaders/
python3 /boot-src/scripts/brand.py config/bootloaders /boot-src/branding "$VERSION"
lb config >/build/lb-config-$EDITION.log 2>&1 || { tail -40 /build/lb-config-$EDITION.log; exit 1; }
echo "lb build ($EDITION) läuft (Log: Volume /build/lb-build-$EDITION.log) …"
if ! lb build >/build/lb-build-$EDITION.log 2>&1; then
    tail -60 /build/lb-build-$EDITION.log
    exit 1
fi
ISO=$(ls -1 ./*.iso | head -1)
cp "$ISO" "/out/$NAME"
cd /out
sha256sum "$NAME" > "$NAME.sha256"
ls -la "$NAME"
cat "$NAME.sha256"
