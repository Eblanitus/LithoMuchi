# Litospherity: перенос мода на новый Horizon (.hmod) и воды из Java в Horizon

**Статус: план, 2026-11-18. Код не написан. Перенос отложен до I-1 (Д-123).** Порядок от автора:
сначала вода выше моря делается на Fabric по [PLAN-WATER-FABRIC.md](PLAN-WATER-FABRIC.md), потом мод и
вода переносятся в Horizon.

> **Решение автора 2026-11-18 (Д-123).** На новом API Horizon сейчас не переходим: без своего узла графа
> плотности (I-1) мир там выйдет хуже, чем на пине `66f3981` и на Fabric (раздел 2). Horizon остаётся на
> пине `66f3981`, вода делается на Fabric, автор отправляет issue I-1 сейчас. Шаги Г1–Г8 начинаются,
> когда I-1 выйдет в Nightly. Путь Б — только резерв, если I-1 откажут.

Рамки:
- Ядро `litho-core` общее и одно. На обеих платформах один сид и режим дают одинаковые поля и уровни
  воды бит в бит. Платформенный код только вызывает ядро.
- Цель — Nightly `nightly-source/2026-11-18-0b14f8a` (Horizon `0b14f8acfd97…`, horizon-mixins
  `e59da9094f0c…`, Rust 1.99.1). Если к старту выйдет новее, берётся последний Nightly с SDK, и
  факты раздела 1 сверяются заново (шаг Г1).
- Старая линия (пин `66f3981`, `.hnmod`, миры `v1-dev`) не трогается, пока автор её не закроет.
  Миры старого мода на новый Horizon не переносятся (новый мод = новые миры, раздел 7).
- Свои блоки не добавляем. Пещеры не трогаем, кроме затычек под водой (как на Fabric).

## 1. Что изменилось в Horizon (проверено по SDK 0b14f8a)

Источники: `docs/modding` на теге, SDK-релиз `sdk-0b14f8acfd97fa2c750e42795249dcc981178ca4`
(`sdk.json`, `build-data.tar.xz` → `functions.json`, `api-source.tar.xz`), исходники на теге.

**Пакет.** Один `.hmod` (zip): `horizon-mod.toml`, `Cargo.toml` + `Cargo.lock`, `src/`, `data/`, `assets/`.
Нативный мод собирает лаунчер на машине игрока против готового SDK, игра не пересобирается. `.hnmod`,
`horizon-native.json`, свой `horizon-mixin.toml` больше не принимаются. Крейт мода зависит только от
`horizon-mod-api` (не от `horizon-core`), свой `build.rs` запрещён, path-крейты разрешены только внутри
папки мода, крейты crates.io — с `Cargo.lock` (точная версия и SHA-256).

**Главное: наши точки в генерации больше не перехватываются миксином.** Миксин `[[native.mixin]]`
остался, но функцию ниже игрового крейта (`horizon_core`, `horizon_world`) он берёт, только если она
помечена `#[horizon_mod_api_macros::replaceable]` и есть в `functions.json` SDK. В `functions.json`
0b14f8a 13 440 замещаемых функций, **ни одной** в `horizon_core` и `horizon_world`. Значит:

| Наша точка (сейчас) | Где | На 0b14f8a |
|---|---|---|
| `SeededContext::noise` (поля C/E/W/T/H, offset, factor) | `horizon_core` | `E_MIXIN_NOT_REPLACEABLE` |
| `MaterialRuleLibrary::compile_aquifer` (лава у стыков, Д-098) | `horizon_world` | то же |
| `AquiferSamplers::new`, `is_lava` (Д-098) | `horizon_world` | то же |
| `GlobalFluidPicker::compute` (вода, horizon#16422 назвал `position`) | `horizon_world` | то же: #16422 влит, но без пометки им не воспользоваться |
| `Title::click`, `confirm_key_size` (паки в новый мир, К6) | `horizon_client` | функций нет; не нужно (ниже) |
| `native_mods`, `native_register_commands`, `native_perform_command` | `horizon_client` | заменены API: строка в «Моды» из манифеста, `register_commands` |

Второй барьер: даже если пометку дадут, наш шум реализует трейт `horizon_core::noise::Noise`, а крейт
мода от `horizon_core` зависеть не может; `[[native.module]]` разрешён только в `horizon_client`, который
ниже по графу не виден. Поэтому просить надо **API**, а не пометку (раздел 5, I-1).

**Что появилось и пригодится:**
- `world_preset` из датапаков читается (хук `register_preset_editors`; пресет попадает в «Тип мира»
  через тег `data/minecraft/tags/worldgen/world_preset/normal.json`). Выбор режима как на Fabric.
- `data/` мода — обычный пак, работает во всех мирах. Отсюда правило: **не переопределять**
  `minecraft:overworld/*`, только свои `litospherity:*` (свой `noise_settings` в своём пресете).
- Свои биомы `data/<ns>/worldgen/biome/*.json`, biome modifiers (фичи/спавны в ванильные биомы).
- `register_chunk_generator` (свой рельеф в `build_terrain` + выбранные ванильные стадии: поверхность,
  карверы, структуры, фичи) и `register_biome_source` (`noise_biome(quart, seed) -> u16`).
- Своих типов функций плотности, шумов, фич (`Feature`) и хука «после шумовой заливки» в API **нет**
  (поиск по `api-source`).

**Дальний LOD.** `lod_grid` (`horizon-world/src/worldgen/generator.rs:2318`) строит даль из
`final_density` роутера `noise_settings` и `aquifer.compute_substance` и не знает о мод-генераторе
(`mod_terrain`). Значит: рельеф, заданный через граф плотности, в дали виден; рельеф своего
`ChunkGenerator` — нет (даль покажет рельеф `noise_settings`). Вода в дали — только из fluid picker.

**horizon#16424 не исправлен на 0b14f8a:** `noise_based.rs:274–276` по-прежнему пробует picker в
`[0, i32::MAX, 0]` и `[±i32::MAX…]`, быстрый путь `skip_above_y` на месте. По git-логу main коммитов
по #16424 нет. Это важно только для воды в дали (раздел 3, I-4).

## 2. Выбор пути

**Путь А (основной): поля через граф плотности, как на Fabric.** На Fabric поля идут через свой тип
функции плотности `litospherity:field` (Ф3, `tools/fabric_resources.py`). На Horizon нужен тот же тип
через мод-API (issue I-1). Тогда:
- JSON `data/` один и тот же на обеих платформах (генерирует `tools/fabric_resources.py`);
- 3D-форма рельефа (ванильный `jaggedness`, Д-117), шумовые пещеры, высота структур — те же, что на Fabric;
- дальний LOD Horizon видит наш рельеф (он читает тот же граф).

**Путь Б (резерв, Д-123): свой `ChunkGenerator` + `BiomeSource`.** Работает на 0b14f8a без новых
хуков, вода выше моря и лава у стыков в нём ставятся прямо в `build_terrain`. Но игра по-прежнему
считает рельеф по графу `noise_settings`, а наш генератор для неё — чёрный ящик. Отсюда:
- **даль LOD** покажет рельеф `noise_settings`, не наш (раздел 1) — научный режим теряет смысл
  (без дальнего вида он не нужен, слова автора 2026-11-15). Свой LOD через API не нарисовать:
  `submit_custom_geometry` даёт ~4096 квадов за кадр, функции дали в `vanillagen` не замещаются;
- **структуры и точка спавна** берут высоту из графа (`getBaseHeight`, `surface_sample`), не нашу, —
  висят или тонут. У `ChunkGenerator` API нет метода «высота колонки», как в Java;
- **шумовые пещеры** («сыр», «спагетти», «лапша», столбы) — часть `final_density`, их не будет;
  карверы (червяки, каньоны) останутся. Вызвать ванильную формулу из генератора API не даёт;
- **3D-форма рельефа** (ванильный `jaggedness`, нависания) пропадёт;
- **мир не совпадёт с Fabric**, пока Fabric не перевести на такой же свой генератор.
LOD, структуры и спавн — дыры молодого API (их можно попросить: «генератор мода отдаёт колонку для
LOD и высоту для структур»); пещеры и форма гор — цена своего генератора на любой платформе. Путь А
не имеет ни одной из этих проблем. Брать путь Б, только если I-1 откажут.

**Форк Sunrise** не рассматриваем: тот же 26.3, но другой мод-API и меньшая аудитория.

## 3. Вода на Horizon: тот же механизм, что выберет Fabric

На Fabric (PLAN-WATER-FABRIC, раздел 3) вода ставится **заливкой по колонкам сразу после шумовой
заливки** (шаг NOISE) и **герметизацией после карверов** (фича RAW_GENERATION). Запасной путь Fabric —
всё в фиче на шаге FEATURES. Horizon повторяет тот путь, который победит в Ф6-В0, иначе блоки разойдутся.

| Что | Fabric | Horizon (нужно) |
|---|---|---|
| Уровень и дно по колонке | `litho_water_batch` (FFI) | `Generator::water_column` напрямую из ядра (тот же код) |
| Заливка после шума | миксин `NoiseFillMixin` | хук «после заливки шума» — **I-2** |
| Герметизация после карверов | фича `litospherity:water_seal` | свой тип фичи или хук после карверов — **I-3** |
| Вода в дальнем LOD | Distant Horizons строит из того же шага | fluid picker по колонке + починка #16424 — **I-4** |
| Лава ниже −54 только у стыков (Д-098) | миксин на `createFluidPicker` | тот же **I-4** (picker отдаёт и лаву) |
| Лава аквиферов (Д-097) | `aquifers.lava` → `litospherity:lava_aquifer` | тот же JSON через **I-1** |

Без I-4 вода в ближних чанках будет, а в дали её не будет (сейчас даль режет воду выше 64, #16424 п. 3).
Это допустимо как промежуточное состояние, но не как выпуск: автор решает (раздел 7).

## 4. Шаги

Оценки грубые, в днях кодера. Все шаги начинаются после Г0 (Д-123).

### Г0. Предусловия (автор)

- На Fabric закрыты Ф6-В1…В5 (CI зелёный, `water-leaks` 0 нарушений) и ФА-В1 автора. Хеш параметров
  `v1-dev-r2` записан в `docs/DECISIONS.md` и больше не меняется без новой версии.
- **I-1 вышел в Nightly** (без него перенос не начинается, Д-123). Автор отправил I-1 сразу, а I-2…I-4 —
  после ответа на I-1; номера issue записаны в этот файл.

### Г1. Спайк SDK (кодер, ~1 день)

- Поставить `cargo-horizon-mod`, `cargo horizon-mod new litho-spike --template native --horizon <commit>`.
- Проверить и записать в `docs/HORIZON-SDK-NOTES.md`:
  - миксин на `horizon_core::density::graph::seeded::SeededContext::noise` действительно отклоняется
    (`E_MIXIN_NOT_REPLACEABLE`); то же для трёх целей лавы;
  - `world_preset` из `data/` мода появляется в «Типе мира»; есть ли он на выделенном сервере
    (`server.properties` / `level-type`);
  - применяется ли `data/` мода к уже созданным ванильным мирам (если да — правило раздела 1 обязательно);
  - собирается ли мод с path-крейтом `litho-core` внутри папки мода и `libm 0.2.16` из `Cargo.lock`;
  - работает ли на Linux-раннере `horizon-launcher native-build --server-output` и headless-сервер
    (нужен для CI Г7). Если нет — CI ограничивается `check`/`fits` и тестами ядра.
- **Готово, если** заметки есть и на каждый пункт ответ «да / нет / как обойти».

### Г2. Каркас `.hmod` (кодер, ~2 дня)

Новый каталог `hmod/` (старый `mod/` остаётся для пина 66f3981):

```text
hmod/
├── horizon-mod.toml   id = "litospherity", api = "0.2", [native] game/horizon/mixins из tools/horizon-revisions.json
├── Cargo.toml         [package] build = false; deps: horizon-mod-api (SDK), litho-core = { path = "crates/litho-core" }
├── Cargo.lock         libm 0.2.16 с SHA-256 (horizon-launcher native-lock hmod)
├── src/lib.rs         #[mod_api::horizon_mod] impl Hooks: register_commands (/litospherity), регистрации из I-1…I-4
└── data/              генерирует xtask (Г3), в git не лежит
```

- `crates/litho-core` в `hmod/` **не копируется руками**: `xtask hmod` кладёт копию при упаковке и
  пишет её хеш; CI сверяет хеш с `crates/litho-core` репозитория (как `check_horizon_pins.py`).
- `tools/horizon-revisions.json`: новые ключи `hmod_horizon`, `hmod_mixins`, `hmod_sdk` рядом со старыми.
- `/litospherity` через API `register_commands`: версия мода, версия генератора, хеш параметров.
- **Готово, если** `cargo horizon-mod check` в `hmod/` проходит, `.hmod` собирается, игра его грузит
  (строка в «Моды», `/litospherity` отвечает). Рельефа ещё нет.

### Г3. Ресурсы: пресеты и `noise_settings` (кодер, ~1 день)

- `tools/fabric_resources.py` делается общим: второй выход `--horizon <out>` пишет то же дерево в
  `hmod/data/`. JSON одинаковы на обеих платформах, кроме различий формата, которые найдёт Г1.
- Содержимое: `worldgen/world_preset/litospherity/{gameplay,scientific}.json` +
  `tags/worldgen/world_preset/normal.json`; `worldgen/noise_settings/litospherity/*`; свои
  `worldgen/density_function/litospherity/<режим>/*` вместо переопределения `minecraft:overworld/*`;
  свой `dimension_type` научного режима (высота 512, Д-113).
- Пресеты сразу на `v1-dev-r2` (как на Fabric, решение автора 2026-11-18, п. 2). Других типов мира нет.
- Файлы Mojang в git не кладутся: берутся из `server.jar` при упаковке (как сейчас для Fabric).
- **Готово, если** оба пресета видны в «Типе мира» и мир создаётся (пока с заглушкой полей: до I-1
  узел `litospherity:field` неизвестен — проверять с ванильным `noise` на его месте).

### Г4. Поля и лава аквиферов через I-1 (кодер, ~3 дня, после влива I-1)

- Зарегистрировать тип `litospherity:field` (параметр `field`: c/e/w/t/h/pv/offset/factor/…) и
  `litospherity:lava_aquifer`. Реализация зовёт `Generator` ядра пакетом по колонке и кэширует по чанку,
  как `LithoNoise::add_to_volume` в `mod/glue/litospherity_glue.rs.in` сейчас. Вызов на каждый блок запрещён.
- Генераторы по `(сид, версия, режим)` — общий реестр, как в старом клее (`generator_for`).
- **Готово, если:**
  - тест Rust: значения поля через путь мода совпадают с `litho-ffi` и `golden/` побитно на 5000 точках
    обоих режимов (как паритет Ф4);
  - мир каждого режима в игре совпадает с Fabric по тем же координатам (`litho-sandbox water-points` даёт
    точки, автор сравнивает в ГА-1);
  - время чанка не хуже старого `.hnmod` на пине 66f3981 больше чем на 15 %.

### Г5. Вода в чанках через I-2 и I-3 (кодер, ~3 дня, после влива I-2/I-3)

- Порт `LithoWater` и `WaterSealFeature` с Fabric на Rust мода: та же логика, тот же
  `Generator::water_column`, те же затычки и ступени (статичный перекат или «живые» — как решил Ф6-В6).
- Если Fabric выбрал запасной путь (всё на FEATURES), то и здесь всё в I-3, без I-2.
- **Готово, если** на headless-сервере (или в мире автора, если Г1 сказал «нет») метрики PLAN-WATER-FABRIC
  Ф6-В5 проходят с теми же порогами: протечки 0, сухие русла ≤ 1 %, заделки ≤ 2 на чанк, рост текущей
  воды ≤ 5 %; и уровни воды в колонках совпадают с Fabric побитно (`water_column` один и тот же).

### Г6. Вода и лава в дали через I-4 (кодер, ~2 дня, после влива I-4)

- Fluid picker по колонке: выше 63 — наш уровень из `water_column`, ниже −54 — лава только у стыков
  (Д-098), иначе ваниль. Один кэш на чанк, без вызова ядра на каждую пробу.
- **Готово, если** в LOD на 512 чанков видно горные озёра и реки выше 64 (проверка ГА-2), а пробы
  `i32::MAX` не ломают ничего (I-4 их убрал или наш picker отвечает на них ванилью).

### Г7. CI (кодер, ~2 дня, параллельно с Г2–Г6)

Новый workflow `.github/workflows/hmod.yml`:
1. `check-pins`: ключи `hmod_*` в `tools/horizon-revisions.json`; хеш копии `litho-core` в пакете =
   хеш `crates/litho-core`.
2. `sdk-check`: ставит `cargo-horizon-mod`, тянет SDK по пину (Linux-срез `api`, ~30 МБ), запускает
   `cargo horizon-mod check` в `hmod/`. Кэш SDK по коммиту.
3. `parity`: Rust 1.99.1 (как у лаунчера); тест сравнивает поля и `water_column` пути мода с `litho-ffi`
   и `golden/` побитно. Это главная проверка «ядро одно».
4. `package`: собирает `.hmod`, артефакт `litospherity-hmod`.
5. `headless` (если Г1 сказал «да»): `horizon-launcher native-build --server-output`, сервер
   генерирует окна вокруг `water-points` обоих режимов, считает метрики Г5. Порог провала — как на Fabric.
6. `fits-next` (раз в сутки, не блокирует): `cargo horizon-mod fits --build <последний Nightly>` — ранний
   сигнал, что новая сборка Horizon ломает мод. Для мода с `[native]` команда отвечает «не применимо»;
   тогда job пробует собрать мод против нового SDK и пишет результат в отчёт.

Старые job'ы (`horizon-core-contract`, `horizon-mixins-contract`, `hnmod`) живут, пока жива старая линия.

### Г8. Проверки автора

- **ГА-1.** Новый мир каждого режима на свежем Nightly с `.hmod`: «Тип мира» → Litospherity, сид 42.
  Сравнить 3–5 точек из `water-points` с тем же сидом на Fabric: рельеф, биом, вода.
- **ГА-2.** Полёт вдоль горной реки с LOD 512 (`docs/CHECKLIST-LOD-512.md`): видна ли вода в дали, TPS.
- **ГА-3.** Ванильный мир с установленным модом остаётся ванильным.

## 5. Тексты issue для Horizon (отправляет автор)

Заголовки в формате команды Horizon «mod API: …». Issue подаются в `horizon-rust/horizon`, а PR с кодом
мод-API с 15.11.2026 идут в форк `arismoko/horizon` (решение проекта, комментарий в horizon#13637). Там же
видно: мейнтейнеры за API вместо правки исходников (мод horizon-mods#474 отклонён за переписывание
исходников при сборке); свои типы фич (I-3) просит и порт Biomes O' Plenty. Отправлять по одному, I-1 первым: без него остальное
не нужно. Тексты на английском, готовы к копированию. Номера после отправки вписать сюда.

### I-1. mod API: custom density function types for worldgen

> **Use case.** Litospherity is a world-generation mod: tectonic plates drive continentalness, erosion,
> ridges, temperature, humidity and the terrain offset/factor of the Overworld router. On pin 66f3981 we
> did this with a cancellable HEAD mixin on `horizon_core::density::graph::seeded::SeededContext::noise`.
> On 0b14f8a that target is below the game crate and not in `functions.json`, and our `Noise` impl cannot
> live in a mod crate (it may not depend on `horizon-core`).
>
> **Ask.** A registrar method like NeoForge/Fabric's `Registries.DENSITY_FUNCTION_TYPE`:
> `registrar.register_density_function_type("field", impl DensityFunctionType)` so that a data pack node
> `{"type": "mymod:field", ...}` compiles to the mod's sampler. We need: the node's JSON arguments, the
> world seed at compile time, `min_value`/`max_value`, a per-point `compute(x, y, z) -> f64`, and a batched
> column/volume fill (like `Noise::add_to_volume`) so the router does not call the mod per block. It must
> run on worldgen worker threads **and in the far-terrain sampler (`lod_grid`)**, which evaluates the same
> router.
>
> **Why not a marker.** `#[replaceable]` on `SeededContext::noise` alone does not help: the helper must
> return `Arc<dyn horizon_core::noise::Noise>`, which a mod crate cannot implement.
>
> Not only us: TerraForged-Horizon registers its own density nodes (`reterraforged:cell`, `reterraforged:noise`,
> `reterraforged:clamp_to_nearest_unit`) through a mixin on the density registry (its
> docs/horizon/host-density-registry.md), so it hits the same wall on the new SDK.
> Related: #13637 (opt-in native worldgen APIs for TerraForged) — this is a smaller, separate piece.

### I-2. mod API: write blocks after the noise fill of `minecraft:noise` levels

> **Use case.** Rivers and lakes above sea level. The terrain is the vanilla noise generator; we need to
> put water into air columns of a chunk right after the noise fill (`NoiseBasedChunkGenerator.doFill`)
> and before the surface rule, so the bed gets underwater surface blocks. The aquifer route does not fit:
> it samples on a 16×12×16 grid, makes stone barriers on river steps and floods caves under high lakes.
>
> **Ask.** A worker-thread hook per chunk, after the noise fill and before `surface`, with the same
> `ChunkAccess` a `ChunkGenerator` gets plus reading blocks (`get_block`) and the level id, so a stateless
> mod can fill columns. The WG heightmaps should be re-primed after it, as `mod_terrain::fill` does.

### I-3. mod API: custom feature types (or a hook after carvers)

> **Use case.** Seal water above sea level against carver cuts: after carvers, for each water block we
> wrote, close air neighbours that our generator says are dry. On Fabric this is a `Feature` at
> `RAW_GENERATION` added to all Overworld biomes.
>
> **Ask.** `registrar.register_feature_type(...)` usable from `placed_feature` JSON and biome modifiers,
> with read/write access to the 3×3 decoration region (`WorldGenLevel` subset: `get_block`, `set_block`,
> heightmaps). A per-chunk hook after `carvers` with the same access would also do.

### I-4. mod API: per-column fluid picker for the Overworld (and horizon#16424)

> **Use case.** The far terrain (`lod_grid`) gets water only from the aquifer's fluid picker, so water
> placed by I-2 never appears in LOD. We also need lava below −54 only near plate boundaries.
>
> **Ask.** A registrar hook to supply the level's global fluid picker: `compute([x, y, z]) -> (level,
> fluid)`, used by the noise aquifer and by `lod_grid`. Thanks for #16422 (named `position`); without a
> `#[replaceable]` mark or an API it cannot be used from a mod on 0b14f8a. Please also land the #16424
> items: no `i32::MAX` probes in `noise_based.rs:274–276`, `skip_above_y` respecting a per-column level,
> and far-terrain bands that reach water above `sea_level + 64`.

## 6. Риски

- **Horizon не даст I-1 или затянет.** Тогда перенос стоит; путь Б только по решению автора (раздел 2).
  Меньший шаг, который можно просить: `#[replaceable]` на `SeededContext::noise` **и** публичный
  `horizon_core::noise::Noise` в `mod_api` — проверить в Г1, решает ли это проблему типов.
- **API сделают «не так»** (без пакетного заполнения или не в `lod_grid`). Тогда скорость или даль хуже
  старого мода. Замер Г4 (≤ +15 %) и ГА-2 это ловят; писать в тот же issue.
- **Мод с `[native]` привязан к одной сборке.** Каждый новый Nightly требует пересборки и проверки
  (`cargo horizon-mod update`). Job `fits-next` даёт ранний сигнал; выпускать мод под конкретный Nightly.
- **Компилятор лаунчера 1.99.1 против нашего stable.** Ядро считает через `libm`, поэтому должно совпадать;
  `parity` в CI гоняется именно на 1.99.1.
- **`data/` мода во всех мирах.** Переопределение `minecraft:*` испортит ванильные миры — запрещено правилом
  раздела 1, проверка ГА-3.
- **Разные точки вставки воды на Fabric и Horizon** дадут разные блоки у берегов. Правило: Horizon
  повторяет путь Fabric (раздел 3); если I-2 не дадут, обе платформы переводятся на путь «всё на FEATURES».
- **Старые миры.** `.hnmod`-миры `v1-dev` на новом Horizon не откроются с нашим генератором (мод не
  грузится). Это решение автора, не баг (раздел 7).
- **Оценка:** Г1–Г4 и Г7 — около 2 недель кодера после влива I-1; Г5–Г6 — около 1 недели после I-2…I-4.

## 7. Решения автора

Принято 2026-11-18 (Д-123): на новый API сейчас не переходим; Horizon на пине `66f3981`; вода на Fabric;
I-1 отправить сейчас; путь Б — резерв.

Открыто:
1. Когда закрывать старую линию (пин `66f3981`, `.hnmod`): сразу после ГА-1 или позже.
2. Если I-1 откажут: идти путём Б (без нашего рельефа в дали, без шумовых пещер, со структурами не на
   своей высоте) или ждать.
3. Можно ли выпустить Horizon-версию с водой только вблизи (без I-4), или ждать I-4.

## 8. Файлы

| Файл | Что |
|---|---|
| `hmod/horizon-mod.toml`, `hmod/Cargo.toml`, `hmod/Cargo.lock`, `hmod/src/*.rs` | новый мод |
| `tools/xtask` (команда `hmod`) | копия `litho-core`, `data/`, упаковка `.hmod` |
| `tools/fabric_resources.py` | общий генератор `data/` для Fabric и Horizon |
| `tools/horizon-revisions.json`, `tools/check_horizon_pins.py` | пины `hmod_*`, хеш копии ядра |
| `.github/workflows/hmod.yml` | CI Г7 |
| `docs/HORIZON-SDK-NOTES.md` | итог Г1 |
| `docs/DECISIONS.md` | записи: путь А/Б, точка воды, закрытие старой линии |
