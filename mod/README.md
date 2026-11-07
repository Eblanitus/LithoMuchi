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
