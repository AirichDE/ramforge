# RAMForge boot edition builder: Debian 12 live-build (Secure Boot via the
# Microsoft-signed shim from Debian), Rust for the Linux binary, QEMU + OVMF
# for boot tests (with and without Secure Boot).
FROM debian:bookworm

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      live-build debootstrap squashfs-tools xorriso mtools dosfstools fdisk cpio \
      grub-efi-amd64-bin grub-pc-bin syslinux-common isolinux \
      ca-certificates curl build-essential pkg-config \
      qemu-system-x86 qemu-utils ovmf python3 \
    && rm -rf /var/lib/apt/lists/*

RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
ENV PATH=/root/.cargo/bin:$PATH
WORKDIR /work
