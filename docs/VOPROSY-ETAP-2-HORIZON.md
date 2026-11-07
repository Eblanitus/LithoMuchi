# Вопросы к Этапу 2 (Horizon мод)

**От:** агент (помощник по коду)  
**К:** пользователю  
**Дата:** 2026-11-07  
**Ветка:** `arena/3e69ffe1-lithomuchi`

---

## Ответы пользователя (2026-11-07)

| № | Вопрос | Ответ | Статус |
|---|--------|-------|--------|
| 1 | Версия игры `26.3`? | Да | ✅ Подтверждено |
| 2 | Коммит Horizon `66f3981`? | Да | ✅ Подтверждено |
| 3 | Название функции генерации чанка | Не знаю | ❌ Неизвестно |
| 4 | Точки входа (injection targets) | Не знаю | ❌ Неизвестно |
| 5 | Типы `DensityVolume` / `DensityError` | Не знаю | ❌ Неизвестно |
| 6 | Папка для модов | Сам засуну | ✅ Самостоятельно |
| 7 | Как измерить время чанка | Не знаю | ❌ Неизвестно |
| 8 | Документация Horizon | Сам ищу | ✅ Самостоятельно |

---

## Осталось неизвестным (нужно для кода мода) — исследовано по образцу `distant_horizons`

### 1. Название функции генерации чанка
**Неизвестно из примера.** В `distant_horizons` нет инъекции в `generate_chunk`. Вместо этого используются:
- `game.streaming.lod_source()` — доступ к генератору
- `game.world.chunks` — загруженные чанки
- `game.streaming.center`, `game.streaming.radius` — центр и радиус стриминга

Нужно искать в коде Horizon версии `66f3981`.

### 2. Дополнительные точки входа (`[[injection]]`)
**Найдены из примера `distant_horizons/mixins/distant_horizons/horizon-mixin.toml`:**

| id | kind | target |
|----|------|--------|
| `frame` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_frame` |
| `distance` | RETURN | `horizon_client::game::Game<'_>::terrain_distance` |
| `list` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_mods` |
| `settings` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_settings` |
| `click` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_settings_click` |
| `command` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_command` |
| `hud` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_hud` |

### 3. Типы Horizon (вместо DensityVolume/DensityError)
**Не найдены в примере.** В `distant_horizons` нет типов `DensityVolume`/`DensityError`. Вместо них используются:
- `horizon_client::blocks::BlockFacts`
- `horizon_client::chunk::Chunk`
- `horizon_world::registry::BlockStateId`
- `horizon_client::lod::TileKey`
- `horizon_client::mesher::{OPAQUE_LAYER, SectionMesh, TerrainVertex}`
- `horizon_client::streaming::{LodSampler, LodSource, WorkerContext}`
- `horizon_core::position::BlockPos`

Для версии `66f3981` типы могут отличаться — нужно проверять в коде Horizon.

### 4. Как измерить время чанка
**Найдено из примера.** Используется `std::time::{Duration, Instant}`:

```rust
const SCAN_EVERY: Duration = Duration::from_millis(250);
const REREAD_EVERY: Duration = Duration::from_secs(5);
const UPLOAD_BUDGET: Duration = Duration::from_millis(2);
```

Измерение через `Instant::now()` до и после операции, затем `.elapsed()`.

---

## Как отвечать

Можно ответить списком цифрами (1–4), короткими фразами или прислать скриншоты из лаунчера Horizon. Ответы помогут записать точные значения в `horizon-native.json` и `horizon-mixin.toml`, после чего можно приступать к коду.
