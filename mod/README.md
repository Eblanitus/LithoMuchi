# Мод 0.2.0 — текущая инструкция

Установить только `.hnmod`, создать новый Normal-мир, открыть Data Packs и
выбрать один из двух встроенных наборов. Отдельная установка ZIP не нужна.
Smoke в обычную поставку не входит. Подробности и границы проверки:
[docs/INTEGRATION-0.2.0.md](../docs/INTEGRATION-0.2.0.md).

Сборка: `cargo run -p xtask -- bundle --out out/bundle`, затем
`python tools/package_mod.py`. Результат — только `out/release/litospherity.hnmod`, оба режима уже внутри.
Устаревшие ZIP режимов удаляются при повторной упаковке. Только для разработки
можно отдельно получить smoke ZIP флагом `--include-smoke`; CI его не публикует.

Ниже сохранена история подключения 0.1.0; её ручная установка датапаков
не требуется для нового мода и нового мира.

---

# Подключение генератора к Horizon 66f3981

Актуальная сборка объединяет клиентские хуки `mod/litospherity/` и worldgen из
шаблонов `mod/glue/` / `mod/horizon-mixin.toml.in`. Не архивируйте одну лишь
папку `mod/litospherity`: без сгенерированных файлов в ней нет генератора.

```bash
cargo test --workspace
cargo run -q -p xtask -- bundle --out out/bundle
cargo run -q -p xtask -- bundle-check --out out/bundle
python tools/package_mod.py
```

В `out/release/` появится только `litospherity.hnmod`.
Его публикует CI в артефакте `litospherity-hnmod`.
Мод содержит `litho-core`, модуль `litospherity_glue` в horizon-core и
cancellable HEAD на `SeededContext::noise`, а также клиентскую команду и строку
в списке модов. Только шумы с пространством имён `litospherity:` заменяются;
обычные шумы Horizon продолжают работать как прежде.

## Проверка на Windows

1. Установите `litospherity.hnmod` через лаунчер Horizon для ревизии
   `66f3981f522b24c250b21523a70080965b940424` и игры 26.3. Разрешение на нативный
   код давайте только после проверки исходников. Дождитесь нативной сборки.
2. Проверьте `/litospherity`: команда сообщает наличие хука, а не факт того,
   что датапак активен.
3. Создайте **новый тестовый мир**, включив только `litospherity_smoke.zip`.
   Ожидается постоянная континентальность 0.8 (суша).
4. В другом новом мире включите ровно один пак:
   `litospherity_v0_gameplay.zip` или `litospherity_v0_scientific.zip`.
   ZIP содержит `pack.mcmeta` в корне и устанавливается как обычный датапак.
5. Сверьте координаты с `litho-sandbox probe`, проверьте скорость чанков и LOD.
   Не включайте сразу несколько наших паков и не применяйте к ценному миру.

**Проверено здесь:** исходные сигнатуры Noise, DensityVolume и SeededContext
по точной ревизии Horizon; структура архива и JSON датапаков. В тестовом крейте
добавлена компиляция того же glue-файла против снимка контракта Noise, проверки
bulk/scalar, масштаба, хвоста буфера и отказа на коротком буфере.
**Не проверено здесь:** запуск Rust-тестов (нет toolchain), применение миксина
лаунчером и генерация в игре. Снимок контракта не заменяет сборку Horizon.
Лава, смещение высоты и автоматический выбор режима через интерфейс не входят
в это подключение. Бюджет производительности ещё требует игровых замеров.

---

## Предыдущие заметки о шаблонах (исторический контекст)

# Шаблоны нативного мода Horizon (Э2, T70–T72)

**Статус: подготовлено, но НЕ проверено сборкой.** В этой среде нет Windows,
`horizon-launcher` и копии Java Edition 26.3, поэтому ни один пункт 5.7 плана
не подтверждён запуском. Всё, что здесь лежит, — эскизы по разделам 5, 10
`docs/PLAN.md` и документации `horizon-mixins`; закрывает Э2 только успешная
сборка лаунчером на Windows. Подробности и чек-лист проверки — `docs/HORIZON.md`.

Файлы с суффиксом `.in` — шаблоны с плейсхолдерами `@VERSION@`, `@MODE@`,
`@NOISE@`, `@HORIZON_REV@`, `@LABEL@`. Их подставляет `xtask`, руками не править.

```text
horizon-mixin.toml.in        точка №1: cancellable HEAD на SeededContext::noise,
                             [[crate]] litho-core с dependent = "horizon-core"
glue/noise_hook.rs.in        помощник: ровно одна приватная функция без атрибутов
glue/litospherity_glue.rs.in LithoNoise, разбор имён, реестр генераторов,
                             ConstNoise для smoke-пака
packs/pack.mcmeta.in         метаданные датапака формата 26.3 (min/max 121)
packs/density_function.json.in  переопределение minecraft:overworld/<поле>
packs/noise.json.in          фиктивное определение шума; диапазон с запасом
```

## Что собирает `xtask`

```bash
cargo run -q -p xtask -- pack --out out/packs
cargo run -q -p xtask -- bundle --out out/bundle
cargo run -q -p xtask -- bundle-check --out out/bundle
```

- `pack` генерирует **три** пака: `litospherity_v0_scientific`,
  `litospherity_v0_gameplay` и `litospherity_smoke`. В каждом — `pack.mcmeta`,
  пять переопределений `overworld/{continents,erosion,ridges,temperature,
  vegetation}` и пять фиктивных определений наших шумов (план 10.4).
  Файлы Mojang не копируются: JSON написаны нами и содержат только ссылки.
- `smoke` — тестовый пак из DoD Э2: все пять полей ссылаются на
  `litospherity:smoke/<поле>`, клей возвращает константу, `continents = 0.8`.
  Если мир создаётся всюду сушей — точка №1 точно работает.
- `bundle` складывает готовый `horizon-mixin.toml`, копию `crates/litho-core`,
  два glue-файла без `.in` и все паки в одну папку. Архивы `.zip` и сам
  `.hnmod` собираются на Windows (T75) — zip без внешних крейтов здесь не
  делается намеренно, чтобы не выдать непроверенный артефакт за готовый.
- `bundle-check` — жёсткая проверка без `|| true`: все файлы на месте, ни одного
  неподставленного `@…@`, JSON начинается и заканчивается фигурной скобкой,
  манифест содержит `dependent = "horizon-core"`. Выполняется в CI.

## Ограничения, которые проверяет T72

- cancellable HEAD с привязками `self`, `graph`, `id` и типомhelperа;
- видимость `crate::litospherity_glue` из `seeded.rs`;
- точные пути `crate::noise::NanRange`, `DensityVolume`, `DensityError`;
- `add_to_volume` по колонкам (в шаблоне оставлен комментарием, не заглушкой,
  которая молча даст неверный рельеф);
- диапазон фиктивного определения шума: граф берёт его из JSON, а не из
  `range()` экземпляра (5.3), `base_amplitude = 4` выбран с запасом под [−1, 1].

### Исправление E_MANIFEST: unknown field `crate`

Пин `horizon-mixins` в `horizon-native.json` обновлён с `0d6b103` (ещё без
поддержки `[[crate]]`) на `5a64af9` (поддержка bundle crates начиная с 0.6.0).
Ревизия самой игры Horizon не меняется. CI теперь запускает настоящий
`horizon-mixins plan` для распакованного `.hnmod` против исходников Horizon,
и только после успешного планирования публикует артефакт. Эта проверка не
заменяет нативную компиляцию и запуск игры.
