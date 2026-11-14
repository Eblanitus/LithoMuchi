# Шаблоны нативного мода Horizon (Э2, T70–T72)

**Состояние на 2026-11-11:** шаблоны больше не эскизы. `xtask` подставляет
пины, `tools/package_mod.py` собирает настоящий `.hnmod` со всеми паками, а CI
применяет его к закреплённым исходникам Horizon через реальный
`horizon-mixins plan` и проверяет density graph 26.3 (`depth(80) = 0.375 +
offset`). Тест №2 в игре подтвердил К2; полная приёмка v1, T142/T143 и релиз
ещё не сделаны. Актуальное состояние — `docs/PLAN.md`, карта
документов — `docs/README.md`, справка по Horizon — `docs/wiki/horizon.md`.

Пины: `tools/horizon-revisions.json` — Horizon `66f3981`, mixins `5a64af9`,
данные 26.3 `5f312935`, libm `0.2.16`. Сверка копий — `tools/check_horizon_pins.py`.

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

## Что собирает `xtask` и упаковщик

```bash
cargo run -q -p xtask -- pack --out out/packs
cargo run -q -p xtask -- bundle --out out/bundle
cargo run -q -p xtask -- bundle-check --out out/bundle
python3 tools/package_mod.py          # .hnmod + ZIP всех паков + native-plan.json
```

- `pack` генерирует паки `v0`, `v1-dev` и `smoke`. В паках v0/v1-dev —
  `pack.mcmeta`, переопределения полей `overworld/*` и фиктивные определения
  наших шумов (план 10.4). Файлы Mojang не копируются: JSON написаны нами и
  содержат только ссылки.
- `v1-dev` до теста №2 переопределял `pv` → `ridges_folded` и `offset`;
  `factor`/`jaggedness` считает ваниль. Экспериментальные паки `v1-dev-i2`
  из `arena/cc2bb388` в рабочий `.hnmod` **не входят**.
- `smoke` — тестовый пак из DoD Э2: все пять полей ссылаются на
  `litospherity:smoke/<поле>`, клей возвращает константу, `continents = 0.8`.
- `bundle` складывает готовый `horizon-mixin.toml`, копию `crates/litho-core`,
  два glue-файла без `.in` и все паки в одну папку. Архивы `.zip` и сам
  `.hnmod` собираются `tools/package_mod.py` в CI; артефакт шага 5 —
  `litospherity-hnmod` id `13857022266` (run `43025888764`), 421527 байт.
- `bundle-check` — жёсткая проверка без `|| true`: все файлы на месте, ни одного
  неподставленного `@…@`, JSON начинается и заканчивается фигурной скобкой,
  манифест содержит `dependent = "horizon-core"`, v0 не содержит новых
  переопределений, v1-dev не содержит factor/jaggedness/I2. Выполняется в CI.

## Ограничения, которые проверяет T72 и реальный CI

- cancellable HEAD с привязками `self`, `graph`, `id` и типом helper'а;
- видимость `crate::litospherity_glue` из `seeded.rs`;
- точные пути `crate::noise::NanRange`, `DensityVolume`, `DensityError`;
- `add_to_volume` по колонкам: Z/X/Y additive, диапазоны Offset `[-2,2]`,
  Factor `[0.5,6.5]`, Jaggedness `[0,1]`;
- диапазон фиктивного определения шума: граф берёт его из JSON, а не из
  `range()` экземпляра (5.3), `base_amplitude = 4` выбран с запасом под [−1, 1];
- хуки написаны под Rust 2024: `gen` зарезервирован, в клее его нет;
- Windows-лаунчер в песочнице по-прежнему не запускается, но реальные типы
  Horizon, настоящий план mixins и граф 26.3 проверяются отдельными job'ами CI,
  а игру подтвердил автор в тесте №2.
