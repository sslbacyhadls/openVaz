# OpenVaz

Панель климата для Lada на микрокомпьютере (Orange Pi). Сенсорный экран вместо штатного блока
климата: пять заслонок на сервоприводах, плавный вентилятор, запрос кондиционера в ЭБУ, чтение
температуры охлаждающей жидкости и кодов ошибок по K-Line. Первая машина: Lada Kalina 2010, бензин.

Программа одна, на Rust, на весь экран, без рабочего стола. Сейчас она работает в QEMU
(обычная ARM-машина, корень только для чтения), а автомобиль заменяет эмулятор на хосте.

## Быстрый старт

Нужны `qemu-system-aarch64`, `rustc` и `cargo` с целью `aarch64-unknown-linux-musl`,
`python3`, `parted`, `mke2fs`, `cpio`, `gzip`, `unshare`.

```sh
cargo fetch
cargo test --no-default-features
emu/build-image.sh
emu/run.sh
```

Во втором терминале:

```sh
emu/car.sh
```

`cargo fetch` нужен один раз: скрипты в `emu/` собирают с `--offline` и без локального кэша
зависимостей падают. Подробно: [docs/emulation.md](docs/emulation.md).

## Документация

Всё лежит в [docs/](docs/README.md):

| Документ | О чём |
|---|---|
| [project.md](docs/project.md) | Исходная идея и требования |
| [questions.md](docs/questions.md) | Уточняющие вопросы и ответы: плата, машина, заслонки, ЭБУ, экран |
| [module.md](docs/module.md) | Железо: схема соединений, каналы, поведение, список деталей |
| [emulation.md](docs/emulation.md) | Как собрать образ и запустить панель в QEMU |
| [plan-january.md](docs/plan-january.md) | Следующий этап: прошивки Итэлма M74 и Январь 7.2, экран показаний, модель двигателя |
| [tasks/](docs/tasks/README.md) | Нарезанные задачи |

## Устройство кода

| Файл | Что внутри |
|---|---|
| `src/main.rs` | Запуск панели: окно, `--fb /dev/fb0` или снимок `--dump-ppm` |
| `src/model.rs` | Состояние климата, сохранение настроек, связь с ЭБУ, жесты |
| `src/layout.rs` | Раскладка экрана 1920×1080 и попадания по кнопкам |
| `src/paint.rs` | Отрисовка экранов (tiny-skia, fontdue) |
| `src/fb.rs` | Вывод в framebuffer и ввод через evdev (на плате и в QEMU) |
| `src/window.rs` | Окно на хосте (winit, softbuffer), фича `window` |
| `src/link.rs` | Текстовый порт панель ↔ автомобиль |
| `src/vehicle.rs`, `src/bin/car.rs` | Эмулятор автомобиля на хосте |
| `emu/` | Сборка диска QEMU, init гостя, запуск |
