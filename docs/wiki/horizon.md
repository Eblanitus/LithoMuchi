# 7. Horizon: точки внедрения, сборка, вода

Статус: **верно** для фиксированной версии (`horizon` `66f3981`, mixins `5a64af9`,
26.3 `5f312935`, libm `0.2.16`). При смене коммита проверять заново.

## Точки внедрения

- Главная: cancellable HEAD на `horizon_core::density::graph::seeded::SeededContext::noise`
  (`graph`, `id`). Возвращает свой `Noise` для имён `litospherity:*`.
- Trait `Noise`: `range()`, `get2`, `get3`, `add_to_volume`. Переопределять `add_to_volume`
  по колонкам.
- Датапаки: поля роутера ссылаются на `minecraft:overworld/{temperature, vegetation,
  continents, erosion, ridges, depth}`. Достаточно переопределить 5 функций плотности.
- Лава: аквиферы (`aquifers` в `noise_settings`); глобально `y < min(−54, sea_level)`.
- Точка №4 `GlobalFluidPicker` — const fn, создаётся в `Pipeline<R>::new`, не принимает
  mixins. Для воды выше моря это главный открытый вопрос (T142).

## Сборка мода

- Крейт мода: только крейты из `Cargo.lock` Horizon (serde, serde_json, toml, glam,
  libm, png). Без rayon, image, noise, без path и git-зависимостей.
- Лаунчер собирает Rust 1.99.1, Windows x86_64-pc-windows-gnu. Первая сборка 20–30 мин.
- Smart App Control на Windows ломает сборку (os error 4551).
- В CI собирается настоящий `.hnmod` (`tools/package_mod.py`). Для игры брать артефакт
  последнего зелёного CI на `main`.

## Вода

- Вода выше уровня моря: механизма не найдено. Развилка R2: вариант B в
  [../archive/T142-DECISION.md](../archive/T142-DECISION.md), решение Д-079.
- Холодные озёра ждут этого механизма (Д-070).

## Ограничения Horizon

- Mod-API: нативный мод — mixin-бандл, правящий исходники перед сборкой
  (HEAD, BEFORE_CALL, RETURN, MODIFY_ARG, OVERWRITE).
- Датапаки с worldgen работают; `world_preset` не читается; новые биомы отбрасываются.
- Спавн ищется в радиусе 2048 от (0, 0).

Связанные: [relief.md](relief.md), [modes-and-versions.md](modes-and-versions.md).
