# План Этапа 2: создание Horizon мода Litospherity

**Статус:** черновик (не начат)  
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

### Шаг 1. Подготовить папку мода

На компьютере пользователя в `C:\LithoMuchi\` создать папку `litospherity/`. В ней будут все файлы мода.

### Шаг 2. Написать `horizon-native.json`

Паспорт мода. По образцу `distant_horizons/horizon-native.json`:

```json
{
  "game": "26.3",
  "horizon": "66f3981",
  "id": "litospherity",
  "schema": 1,
  "version": "0.1.0"
}
```

✅ Оба значения (`26.3` и `66f3981`) подтверждены пользователем.

### Шаг 3. Написать `horizon-mixin.toml`

Главный конфиг мода. По образцу `distant_horizons/mixins/distant_horizons/horizon-mixin.toml`:

```toml
[mod]
id = "litospherity"
version = "0.1.0"
priority = 100

[compatibility]
horizon_revision = "66f3981"

[[module]]
id = "runtime"
file = "crates/horizon-client/src/game.rs"
name = "litospherity"
source = "runtime.rs"

[[module]]
id = "bridge"
file = "crates/horizon-client/src/game.rs"
name = "litospherity_bridge"
source = "bridge.rs"

# Точки входа (по образцу distant_horizons):
[[injection]]
id = "frame"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game<'_>::native_frame"
helper = "frame.rs"
bindings = [
  { name = "game", ty = "&mut crate::game::Game<'_>", value = "&mut *self" },
]

[[injection]]
id = "distance"
kind = "RETURN"
target = "horizon_client::game::Game<'_>::terrain_distance"
helper = "distance.rs"
bindings = [
  { name = "value", ty = "i32", value = "$value" },
]

[[injection]]
id = "list"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game<'_>::native_mods"
helper = "list.rs"
bindings = [
  { name = "mods", ty = "&mut Vec<crate::game::native_hooks::NativeMod>", value = "&mut *mods" },
]

[[injection]]
id = "settings"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game<'_>::native_settings"
helper = "settings.rs"
bindings = [
  { name = "game", ty = "&mut crate::game::Game<'_>", value = "&mut *self" },
  { name = "mod_id", ty = "&str", value = "mod_id" },
  { name = "buttons", ty = "&mut Vec<String>", value = "&mut *buttons" },
]

[[injection]]
id = "click"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game<'_>::native_settings_click"
helper = "click.rs"
bindings = [
  { name = "game", ty = "&mut crate::game::Game<'_>", value = "&mut *self" },
  { name = "mod_id", ty = "&str", value = "mod_id" },
  { name = "button", ty = "usize", value = "button" },
]

[[injection]]
id = "command"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game<'_>::native_command"
helper = "command.rs"
bindings = [
  { name = "game", ty = "&mut crate::game::Game<'_>", value = "&mut *self" },
  { name = "command", ty = "&str", value = "command" },
  { name = "ran", ty = "&mut bool", value = "&mut *ran" },
]

[[injection]]
id = "hud"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game<'_>::native_hud"
helper = "hud.rs"
bindings = [
  { name = "game", ty = "&mut crate::game::Game<'_>", value = "&mut *self" },
  { name = "batch", ty = "&mut horizon_client::hud::HudBatch", value = "&mut *batch" },
  { name = "corners", ty = "&mut horizon_client::extras_hud::Corners", value = "&mut *corners" },
]

# Точка входа генерации чанка (по коду Horizon):
[[injection]]
id = "fill"
kind = "HEAD"
target = "horizon_client::game::native_hooks::Game<'_>::native_fill"
helper = "fill.rs"
bindings = [
  # TODO: привязки для генерации чанка (координаты, семя, настройки мира)
  # Нужно проверить сигнатуру native_fill в коде Horizon 66f3981
]

### Шаг 4. Написать `bridge.rs`

Клей между нашим генератором и Horizon. По образцу `distant_horizons/mixins/distant_horizons/bridge.rs`:

- Знает типы Horizon:
  - Из примера `distant_horizons`:
    - `horizon_client::blocks::BlockFacts`
    - `horizon_client::chunk::Chunk`
    - `horizon_world::registry::BlockStateId`
    - `horizon_client::lod::TileKey`
    - `horizon_client::mesher::{OPAQUE_LAYER, SectionMesh, TerrainVertex}`
    - `horizon_client::streaming::{LodSampler, LodSource, WorkerContext}`
    - `horizon_core::position::BlockPos`
  - Из кода Horizon (`crates/horizon-core/src/noise/volume.rs`):
    - `horizon_core::noise::DensityVolume` — границы и шаги объема плотности
    - `horizon_core::noise::DensityError` — ошибки объема (`NonpositiveDimensions`, `NonpositiveSteps`, `VolumeTooLarge`, `BufferTooShort`)
- Реализует `add_to_volume` по колонкам (уже заготовлено в `mod/glue/litospherity_glue.rs.in`)
- Регистрирует шумовые источники для Horizon
- Компилируется только внутри игры Horizon

### Шаг 5. Написать `runtime.rs` (или использовать `crates/litho-core`)

Наша генерация мира, не зависящая от Horizon. Можно подключить `crates/litho-core` как библиотеку — он уже компилируется, все 72 теста проходят.

### Шаг 6. Собрать `.hnmod`

1. Проверить `cargo check --workspace` (уже проходит)
2. Дописать команды сборки в `tools/xtask` (есть `pack` и `bundle`)
3. Заархивировать папку `litospherity/` в ZIP с расширением `.hnmod`
4. Положить в папку модов лаунчера Horizon

### Шаг 7. Протестировать в игре

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
