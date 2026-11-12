#!/bin/sh
# Эмулятор автомобиля на хосте. Подключается к сокету, который открывает emu/run.sh.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p "$root/emu/build"
name=${1:-итэлма-м74}
cargo build --offline --bin openvaz-car
exec "$root/target/debug/openvaz-car" "$root/emu/build/car.sock" "$name"
