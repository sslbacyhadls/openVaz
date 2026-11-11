#!/bin/sh
# Эмулятор автомобиля на хосте. Подключается к сокету, который открывает emu/run.sh.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p "$root/emu/build"
cargo build --release --bin openvaz-car --offline
exec "$root/target/release/openvaz-car" "$root/emu/build/car.sock" "${1:-B564CD04}"
