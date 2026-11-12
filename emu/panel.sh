#!/bin/sh
# Панель в окне хоста, напрямую связана с emu/car.sh; QEMU не нужен.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release --bin openvaz
exec "$root/target/release/openvaz" "$@"
