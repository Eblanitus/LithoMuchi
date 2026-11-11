# План Этапа 2: создание Horizon мода Litospherity

> **УСТАРЕЛО / исторический план.** Описывает нереализованную интеграцию через
> `native_fill`. Рабочая линия — density-graph noise hook, `tools/package_mod.py`
> и `docs/PLAN-STATUS-2026-11-11.md`. Не использовать как ТЗ.

<!-- ИСТОРИЧЕСКИЙ / PLAN-STATUS-2026-11-11 -->

---


**Статус:** структура мода создана (файлы в `mod/litospherity/`)  
**Образец:** `distant_horizons.hnmod` (распакован в ветке `arena/3e69ffe1-lithomuchi`)  
**Версия Horizon:** `66f3981` (✅ подтверждено пользователем)  
**Версия игры:** `26.3` (✅ подтверждено пользователем)

---

## 1. Структура мода (по образцу `distant_horizons`)

Готовый мод `distant_horizons` имеет такую структуру после распаковки:

```
distant_horizons/
├── horizon-native.json          # паспорт мода
└── mixins/
    └── distant_horizons/
        ├── horizon-mixin.toml   # конфиг модулей и точек входа
        ├── bridge.rs            # клей: знает типы Horizon (компилируется только в игре)
        ├── runtime.rs           # основная логика (не зависит от Horizon)
        ├── frame.rs, distance.rs, list.rs, settings.rs, click.rs, command.rs, hud.rs  # точки входа
        └── ...
```

Наш мод по тому же принципу:

```
litospherity/
├── horizon-native.json          # паспорт мода
└── mixins/
    └── litospherity/
        ├── horizon-mixin.toml   # конфиг модулей и точек входа
        ├── bridge.rs            # клей к Horizon (адаптируем из mod/glue/litospherity_glue.rs.in)
        └── runtime.rs           # наша генерация (может использовать crates/litho-core)
```

---

## 2. Пошаговый план

### ✅ Шаг 1. Подготовить папку мода — ВЫПОЛНЕНО

Папка `mod/litospherity/` создана в репозитории.

На компьютере пользователя в `C:\LithoMuchi\` создать папку `litospherity/`. В ней будут все файлы мода.

### ✅ Шаг 2. Написать `horizon-native.json` — ВЫПОЛНЕНО

Файл `mod/litospherity/horizon-native.json` создан с подтвержденными значениями:
- `game`: `26.3`
- `horizon`: `66f3981`

### ✅ Шаг 3. Написать `horizon-mixin.toml` — ВЫПОЛНЕНО

Файл `mod/litospherity/mixins/litospherity/horizon-mixin.toml` создан со всеми 8 точками входа, включая `fill` для генерации чанков.



### ✅ Шаг 4. Написать `bridge.rs` — ВЫПОЛНЕНО

Файл `mod/litospherity/mixins/litospherity/bridge.rs` создан:
- Типы `horizon_core::noise::{DensityVolume, DensityError}` из кода Horizon
- Типы из примера `distant_horizons`
- Заглушка `add_to_volume_columns` по колонкам (T74)

### ✅ Шаг 5. Написать `runtime.rs` — ВЫПОЛНЕНО

Файл `mod/litospherity/mixins/litospherity/runtime.rs` создан:
- Константы мода (ID, NAME, VERSION, DESCRIPTION)
- Заглушки `init()`, `surface_height()`, `block_density()`

### ✅ Шаг 6. Хелперы точек входа — ВЫПОЛНЕНО

Созданы файлы: `frame.rs`, `distance.rs`, `list.rs`, `settings.rs`, `click.rs`, `command.rs`, `hud.rs`, `fill.rs`

### ⏳ Шаг 7. Собрать `.hnmod` — ОСТАЛСЯ

1. Проверить `cargo check --workspace` на компьютере пользователя
2. Дописать команды сборки в `tools/xtask` (есть `pack` и `bundle`)
3. Заархивировать папку `litospherity/` в ZIP с расширением `.hnmod`
4. Положить в папку модов лаунчера Horizon (пользователь сделает сам)

### ⏳ Шаг 8. Протестировать в игре — ОСТАЛСЯ

- Запустить Minecraft 26.3 через Horizon
- Включить мод `litospherity`
- Проверить, что мир генерируется без вылетов
- Проверить DoD Этапа 3: рост времени генерации чанков ≤ 25 % от ванили
- Измерение времени: по образцу `distant_horizons` — `std::time::{Duration, Instant}`, `Instant::now()` до и после операции, `.elapsed()`

---

## 3. Что уже готово (в ветке `arena/3e69ffe1-lithomuchi`)

| Компонент | Состояние |
|-----------|-----------|
| `crates/litho-core` | Готов, все 72 теста проходят, код компилируется |
| `mod/glue/litospherity_glue.rs.in` | Заготовка клея (нужно адаптировать под Horizon) |
| `tools/xtask pack|bundle` | Инструменты сборки паков есть |
| Исправления после разбора | Точка появления `p*`, реки в W, `add_to_volume`, `render.yml` — влиты |

---

## 4. Риски

| Риск | Что делать |
|------|------------|
| Сигнатура `native_fill` может отличаться в версии 66f3981 | Проверить в коде Horizon при первой сборке |
| Типы `DensityVolume`/`DensityError` могут отличаться | Сверять при первой сборке (задача T72 в плане) |
| Мод не загрузится в лаунчере | Проверить `horizon-native.json` и версию Horizon |
| Скорость генерации чанков превысит бюджет | Оптимизировать `field()` и `add_to_volume` (см. раздел 4 основного разбора) |

---

## 5. Что нужно сделать пользователю перед началом кода

1. ✅ Версия игры `26.3` — подтверждена.
2. ✅ Коммит Horizon `66f3981` — подтвержден.
3. ✅ Название функции генерации чанка — `native_fill` (найдено в коде Horizon).
4. ✅ Типы `DensityVolume` / `DensityError` — `horizon_core::noise::{DensityError, DensityVolume}` (найдены в коде Horizon).
5. ✅ Все точки входа (`[[injection]]`) — найдены в `native-hooks.json`.
6. Узнать, куда класть `.hnmod` в лаунчере Horizon (папка модов).

---

## 6. Связь с планом LithoMuchi

- Этап 2 в `docs/PLAN.md` — задачи T70–T72 (разведка Horizon).
- Этап 3 — задачи T73–T78 (MVP в игре, `.hnmod`, сборки, замеры).
- Этот план раскрывает этапы 2–3 на практике.
