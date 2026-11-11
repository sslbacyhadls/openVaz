#!/bin/sh
# Эмулятор автомобиля на хосте. Подключается к сокету, который открывает emu/run.sh.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p "$root/emu/build"
cargo build --bin openvaz-car --offline
exec "$root/target/debug/openvaz-car" "$root/emu/build/car.sock"
