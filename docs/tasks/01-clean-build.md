# 01. Сборка с чистого клона

- Приоритет: P1
- Размер: S
- Кто:
- Статус: открыта

## Зачем

`emu/build-image.sh` и `emu/car.sh` собирают с `--offline`. На машине, где кэша cargo ещё нет,
сборка падает с `no matching package named fontdue`. Новый участник проекта упирается в это
первым же запуском.

Образ гостя берёт Alpine из `latest-stable`: при следующем релизе Alpine поменяются ядро и
модули, и сборка может сломаться сама по себе (в скрипте жёстко прописаны пути модулей
`virtio_blk`, `ext4`, `crc16`).

## Что сделать

1. В обоих скриптах убрать `--offline` или перед сборкой вызывать `cargo fetch`.
2. Закрепить версию Alpine в `emu/build-image.sh` (например `v3.22` вместо `latest-stable`).
3. Переменные окружения `OPENVAS_STATE_DIR`, `OPENVAS_CAR`, `OPENVAS_KIOSK` переименовать в
   `OPENVAZ_*`, как называется проект (`src/model.rs`, `src/link.rs`, `src/window.rs`,
   `emu/init`).

## Готово, когда

- Свежий `git clone` с пустым `~/.cargo/registry` собирается `emu/build-image.sh` без ручных
  шагов.
- Два запуска `emu/build-image.sh` с разницей в месяц дают одно и то же ядро.
- `grep -rn OPENVAS_` ничего не находит.
