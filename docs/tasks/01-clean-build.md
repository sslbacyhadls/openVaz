# 01. Сборка с чистого клона

- Приоритет: P1
- Размер: S
- Кто: исполнитель ветки feature/tasks-01-04
- Статус: закрыта

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
3. Исправить префикс переменных окружения на `OPENVAZ_STATE_DIR`, `OPENVAZ_CAR`,
   `OPENVAZ_KIOSK`, как называется проект (`src/model.rs`, `src/link.rs`, `src/window.rs`,
   `emu/init`).

## Готово, когда

- Свежий `git clone` с пустым `~/.cargo/registry` собирается `emu/build-image.sh` без ручных
  шагов.
- Два запуска `emu/build-image.sh` с разницей в месяц дают одно и то же ядро.
- Поиск прежнего ошибочного префикса переменных окружения ничего не находит.

## Решения

- Использовать Alpine v3.22 и точный пакет `linux-virt=6.12.117-r0`, доступный при проверке.
  Обновления внутри ветки Alpine не должны молча менять ядро. Если зеркало удалит эту ревизию,
  сборка явно завершится ошибкой; долговременный архив пакетов в эту задачу не входит.
- Полный запуск `emu/build-image.sh` исключён по указанию задачи: нужны сеть и unshare.

## Что сделано

- `emu/build-image.sh`, `emu/car.sh`: убран автономный режим Cargo; в сборке образа
  закреплены ветка Alpine и версия ядра. Путь `crc16` в initramfs и modules.dep
  согласован с выбранным ядром (`kernel/lib/crc16.ko`).
- `src/model.rs`, `src/link.rs`, `src/window.rs`, `emu/init`: единый префикс окружения `OPENVAZ_`.
- Этот файл и `docs/tasks/README.md`: обновлены формулировки и статус.
- `sh -n emu/build-image.sh emu/car.sh emu/init`: успешно.
- URL ветки Alpine и пакета ядра: HTTP 200. Пакет скачан, наличие всех пяти
  модулей initramfs и зависимости ext4 (`jbd2,mbcache,crc16`) проверены по архиву.
- Сборка `cargo build --no-default-features --locked` с новым пустым `CARGO_HOME`
  и отдельным `CARGO_TARGET_DIR`: успешно, зависимости скачаны автоматически.
- `cargo test --no-default-features`: 12 passed, 0 failed.
- `cargo clippy --no-default-features --all-targets`: exit 0; прежние предупреждения
  в model, paint и car оставлены вне этой задачи.
- Поиск прежнего префикса: совпадений нет. Полная сборка aarch64-образа не проверялась.
