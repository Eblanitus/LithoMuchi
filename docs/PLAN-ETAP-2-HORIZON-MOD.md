# План Этапа 2: создание Horizon мода Litospherity

**Статус:** черновик (не开始ан)  
**Образец:** `distant_horizons.hnmod` (распакован в ветке `arena/3e69ffe1-lithomuchi`)  
**Версия Horizon:** `66f3981` (по данным пользователя)  
**Версия игры:** `26.3` (требует подтверждения)

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

**Требует подтверждения у пользователя:**
- Версия игры `26.3` — актуальна ли для его лаунчера Horizon?
- Коммит `66f3981` — правильный ли для его Horizon?

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

[[injection]]
id = "chunk_gen"
kind = "HEAD"
target = "???"   # <-- нужно узнать у пользователя / в коде Horizon
helper = "chunk_gen.rs"
bindings = [ ... ]
```

**Что нужно уточнить:**
- Как называется функция генерации чанка в Horizon (цель `target` для `[[injection]]`)?
- Какие типы данных передавать (координаты чанка, семя, настройки)?

### Шаг 4. Написать `bridge.rs`

Клей между нашим генератором и Horizon. По образцу `distant_horizons/mixins/distant_horizons/bridge.rs`:

- Знает типы Horizon (`horizon_client::chunk::Chunk`, `horizon_world::registry::BlockStateId` и т.д.)
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
| Неизвестны названия функций Horizon | Спросить у разработчиков Horizon или посмотреть в исходном коде примера `distant_horizons` |
| Типы `DensityVolume`/`DensityError` не совпадут | Сверять при первой сборке (задача T72 в плане) |
| Мод не загрузится в лаунчере | Проверить `horizon-native.json` и версию Horizon |
| Скорость генерации чанков превысит бюджет | Оптимизировать `field()` и `add_to_volume` (см. раздел 4 основного разбора) |

---

## 5. Что нужно сделать пользователю перед началом кода

1. Подтвердить версию игры `26.3` в лаунчере Horizon.
2. Подтвердить коммит Horizon `66f3981`.
3. Узнать название функции генерации чанка в Horizon (для `target` в `[[injection]]`).
4. Узнать точные типы `DensityVolume` / `DensityError` в Horizon API.
5. Узнать, куда класть `.hnmod` в лаунчере Horizon (папка модов).

---

## 6. Связь с планом LithoMuchi

- Этап 2 в `docs/PLAN.md` — задачи T70–T72 (разведка Horizon).
- Этап 3 — задачи T73–T78 (MVP в игре, `.hnmod`, сборки, замеры).
- Этот план раскрывает этапы 2–3 на практике.
