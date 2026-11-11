# Вопросы к Этапу 2 (Horizon мод)

> **УСТАРЕЛО / исторические ответы.** Ответы получены по `main` Horizon, а не
> по закреплённой ревизии `66f3981`; рабочие проверки теперь в CI на настоящих
> исходниках. См. `docs/PLAN-STATUS-2026-11-11.md`.

<!-- ИСТОРИЧЕСКИЙ / PLAN-STATUS-2026-11-11 -->

---


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
| 3 | Название функции генерации чанка | `native_fill` | ✅ Найдено в коде Horizon |
| 4 | Точки входа (injection targets) | 8 хуков | ✅ Найдено в коде Horizon |
| 5 | Типы `DensityVolume` / `DensityError` | `horizon_core::noise` | ✅ Найдено в коде Horizon |
| 6 | Папка для модов | Сам засуну | ✅ Самостоятельно |
| 7 | Как измерить время чанка | `Instant::now()` | ✅ Найдено в примере distant_horizons |
| 8 | Документация Horizon | Сам ищу | ✅ Самостоятельно |

---

## Все ответы найдены — исследовано по коду Horizon (horizon-rust/horizon, ветка main)

### 1. Название функции генерации чанка — ✅ `native_fill`
В `native-hooks.json` хук `fill` с target:
```
horizon_client::game::native_hooks::Game<'_>::native_fill
```
Это точка входа для заполнения/генерации чанков террейна.

### 2. Все точки входа (`[[injection]]`) — ✅ найдены в `native-hooks.json`

| id | kind | target |
|----|------|--------|
| `frame` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_frame` |
| `distance` | RETURN | `horizon_client::game::Game<'_>::terrain_distance` |
| `list` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_mods` |
| `settings` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_settings` |
| `click` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_settings_click` |
| `command` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_command` |
| `hud` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_hud` |
| `fill` | HEAD | `horizon_client::game::native_hooks::Game<'_>::native_fill` |

### 3. Типы `DensityVolume` / `DensityError` — ✅ найдены в `crates/horizon-core/src/noise/volume.rs`

```rust
pub enum DensityError {
    NonpositiveDimensions,
    NonpositiveSteps,
    VolumeTooLarge,
    BufferTooShort,
}

pub struct DensityVolume {
    sizes: [usize; 3],
    minimum: [i32; 3],
    steps: [i32; 3],
    size: usize,
}
```

Экспортируются как `horizon_core::noise::{DensityError, DensityVolume}`.

### 4. Измерение времени чанка — ✅ `std::time::{Duration, Instant}`
По образцу `distant_horizons`:
```rust
const SCAN_EVERY: Duration = Duration::from_millis(250);
const REREAD_EVERY: Duration = Duration::from_secs(5);
const UPLOAD_BUDGET: Duration = Duration::from_millis(2);
```
Измерение через `Instant::now()` до и после операции, затем `.elapsed()`.

---

## Источники

- https://github.com/horizon-rust/horizon — официальный репозиторий Horizon
- `native-hooks.json` — каталог всех хуков (точек входа)
- `crates/horizon-core/src/noise/volume.rs` — определения `DensityVolume` и `DensityError`
- `crates/horizon-core/src/density/mod.rs` — использование типов в сэмплерах плотности
- `distant_horizons/mixins/distant_horizons/horizon-mixin.toml` — образец конфига мода (уже в репозитории Litospherity)

---

## Что делать дальше

Все ответы получены. Можно записывать точные значения в `horizon-native.json` и `horizon-mixin.toml` (шаблоны уже в `docs/templates/`), затем приступать к коду `bridge.rs` и `runtime.rs` для мода Litospherity.
