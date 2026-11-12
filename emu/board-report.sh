#!/bin/sh
# Печатает факты для задачи 06. Запускать на Orange Pi, когда панель уже на экране.
set -eu

model="нет"
if [ -r /proc/device-tree/model ]; then
    model=$(tr -d '\0' < /proc/device-tree/model)
fi
echo "плата: $model"

if [ -r /etc/armbian-release ]; then
    board=$(sed -n 's/^BOARD=//p' /etc/armbian-release)
    version=$(sed -n 's/^VERSION=//p' /etc/armbian-release)
    echo "armbian: ${board:-нет} ${version:-}"
else
    echo "armbian: нет"
fi
echo "ядро: $(uname -r)"

if [ -r /sys/class/graphics/fb0/virtual_size ]; then
    name=$(cat /sys/class/graphics/fb0/name 2>/dev/null || echo "fb0")
    size=$(cat /sys/class/graphics/fb0/virtual_size)
    echo "fb0: $name $size"
else
    echo "fb0: нет"
fi

echo "usb:"
lsusb || true
echo "ввод:"
ls -l /dev/input/event* 2>/dev/null || echo "нет event"
echo "порт:"
ls -l /dev/ttyUSB* /dev/ttyACM* 2>/dev/null || echo "нет последовательного адаптера"

if pid=$(pidof openvaz); then
    ps -o rss,pcpu,comm -p "$pid"
else
    echo "openvaz: не запущен"
fi
