#!/bin/sh
# Запускает панель в QEMU: 4 ядра Cortex-A53, 512 МБ, диск из emu/build-image.sh.
# ttyAMA0 — консоль. ttyAMA1 — сокет автомобиля, к нему подключается emu/car.sh.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
sock=$root/emu/build/car.sock
mkdir -p "$root/emu/build"
rm -f "$sock"
echo "порт автомобиля: $sock" >&2
exec qemu-system-aarch64 \
    -machine virt \
    -cpu cortex-a53 \
    -smp 4 \
    -m 512 \
    -kernel "$root/emu/build/vmlinuz" \
    -initrd "$root/emu/build/initramfs.img" \
    -append "console=ttyAMA0,115200 vt.global_cursor_default=0 root=/dev/vda1 rootfstype=ext4 rootflags=ro ro rootwait" \
    -drive if=none,id=hd,format=raw,file="$root/emu/build/disk.img" \
    -device virtio-blk-pci,drive=hd \
    -device virtio-gpu-pci,xres=1920,yres=1080 \
    -device virtio-tablet-pci \
    -device virtio-keyboard-pci \
    -nic none \
    -display sdl,gl=off,show-cursor=on \
    -serial stdio \
    -serial "unix:$sock,server=on,wait=off"
