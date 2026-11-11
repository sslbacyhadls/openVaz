#!/bin/sh
# Собирает диск QEMU: корень только для чтения и раздел настроек.
# Запуск из любой папки: emu/build-image.sh
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"
cache=$root/emu/cache
build=$root/emu/build
rootfs=$build/rootfs
mirror=https://dl-cdn.alpinelinux.org/alpine/v3.24

mkdir -p "$cache" "$build"
rm -rf "$rootfs"
mkdir -p "$rootfs"

export PATH="${HOME}/.cargo/bin:${PATH}"
sysroot=$(rustc --print sysroot)
export PATH="${sysroot}/lib/rustlib/x86_64-unknown-linux-gnu/bin:${PATH}"
cargo build --release --target aarch64-unknown-linux-musl --no-default-features --manifest-path "$root/Cargo.toml"

fetch() {
    python3 - "$1" "$2" <<'PY'
import sys
import urllib.request
url, dest = sys.argv[1], sys.argv[2]
print("загрузка", url, flush=True)
urllib.request.urlretrieve(url, dest)
PY
}

index=$(python3 -c 'import urllib.request,sys; print(urllib.request.urlopen(sys.argv[1], timeout=60).read().decode())' "$mirror/main/x86_64/")
apk_name=$(printf '%s\n' "$index" | sed -n 's/.*\(apk-tools-static-[^"<]*\.apk\).*/\1/p' | head -n 1)
if [ -z "$apk_name" ]; then
    echo "не найден apk-tools-static на $mirror" >&2
    exit 1
fi
if [ ! -f "$cache/$apk_name" ]; then
    fetch "$mirror/main/x86_64/$apk_name" "$cache/$apk_name"
fi
rm -rf "$cache/apk-tools"
mkdir -p "$cache/apk-tools"
tar -xzf "$cache/$apk_name" -C "$cache/apk-tools"
apk=$cache/apk-tools/sbin/apk.static
chmod +x "$apk"

unshare --user --map-root-user "$apk" \
    --arch aarch64 \
    --root "$rootfs" \
    --initdb \
    --no-scripts \
    --allow-untrusted \
    -X "$mirror/main" \
    add busybox musl linux-virt

for app in sh mount mkdir sleep awk dmesg cat ls ln; do
    ln -sfn busybox "$rootfs/bin/$app"
done

kver=$(ls "$rootfs/lib/modules")
config=$rootfs/boot/config-*
echo "ядро $kver"
grep -E '^CONFIG_VIRTIO_BLK=|^CONFIG_EXT4_FS=|^CONFIG_DRM_FBDEV_EMULATION=' $config || true

mkdir -p "$rootfs/usr/bin" "$rootfs/dev" "$rootfs/proc" "$rootfs/sys" "$rootfs/tmp" "$rootfs/run" \
    "$rootfs/var/log" "$rootfs/var/lib/openvaz"
cp "$root/target/aarch64-unknown-linux-musl/release/openvaz" "$rootfs/usr/bin/openvaz"
chmod 755 "$rootfs/usr/bin/openvaz"
cp "$root/emu/init" "$rootfs/init"
chmod 755 "$rootfs/init"
cp "$rootfs/boot/vmlinuz-virt" "$build/vmlinuz"

initrd_dir=$build/initramfs
rm -rf "$initrd_dir"
mkdir -p "$initrd_dir/bin" "$initrd_dir/lib/modules/$kver"
cp -a "$rootfs/bin/busybox" "$initrd_dir/bin/busybox"
for app in sh mount mkdir modprobe sleep switch_root; do
    ln -sfn busybox "$initrd_dir/bin/$app"
done
cp -a "$rootfs/lib/ld-musl-aarch64.so.1" "$rootfs/lib/libc.musl-aarch64.so.1" "$initrd_dir/lib/"
cp "$root/emu/initramfs-init" "$initrd_dir/init"
chmod 755 "$initrd_dir/init"
moddir=$rootfs/lib/modules/$kver
# Ядро собрано без CONFIG_MODULE_DECOMPRESS: модули на диске — gzip,
# а busybox modprobe отдаёт файл ядру как есть. В initramfs кладём уже ELF.
: > "$initrd_dir/lib/modules/$kver/modules.dep"
: > "$initrd_dir/lib/modules/$kver/modules.alias"
: > "$initrd_dir/lib/modules/$kver/modules.builtin"
for rel in \
    kernel/drivers/block/virtio_blk.ko.gz \
    kernel/fs/ext4/ext4.ko.gz \
    kernel/fs/mbcache.ko.gz \
    kernel/fs/jbd2/jbd2.ko.gz \
    kernel/lib/crc/crc16.ko.gz
do
    plain=${rel%.gz}
    mkdir -p "$initrd_dir/lib/modules/$kver/$(dirname "$plain")"
    gzip -dc "$moddir/$rel" > "$initrd_dir/lib/modules/$kver/$plain"
done
cat > "$initrd_dir/lib/modules/$kver/modules.dep" <<'EOF'
kernel/drivers/block/virtio_blk.ko:
kernel/lib/crc/crc16.ko:
kernel/fs/mbcache.ko:
kernel/fs/jbd2/jbd2.ko:
kernel/fs/ext4/ext4.ko: kernel/lib/crc/crc16.ko kernel/fs/mbcache.ko kernel/fs/jbd2/jbd2.ko
EOF
(cd "$initrd_dir" && find . -print0 | cpio --null -o -H newc | gzip -9) > "$build/initramfs.img"

image=$build/disk.img
rm -f "$image"
truncate -s 1400M "$image"
parted -s "$image" mklabel gpt
parted -s "$image" mkpart root ext4 1MiB 1100MiB
parted -s "$image" mkpart data ext4 1100MiB 100%

python3 - "$image" "$rootfs" <<'PY'
import subprocess
import sys

image, rootfs = sys.argv[1:]
text = subprocess.check_output(["parted", "-m", image, "unit", "B", "print"], text=True)
parts = []
for line in text.splitlines():
    if not line[:1].isdigit():
        continue
    number, start, end, size, *_ = line.split(":")
    start = int(start.rstrip("B"))
    size = int(size.rstrip("B"))
    parts.append((int(number), start, size // 4096 * 4096))

def make(number, label, extra):
    _, start, size = next(part for part in parts if part[0] == number)
    blocks = size // 4096
    command = [
        "mke2fs", "-F", "-t", "ext4", "-b", "4096", "-L", label,
        "-E", f"offset={start}",
    ]
    command.extend(extra)
    command.extend([image, str(blocks)])
    subprocess.check_call(command)

make(1, "openvaz-root", ["-d", rootfs])
make(2, "openvaz-data", [])
print("диск", image)
PY
