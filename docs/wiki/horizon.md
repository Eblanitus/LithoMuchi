# 7. Horizon: точки внедрения, сборка, вода

Подробности и история разведки — [спецификация v1.0, разделы 5 и 10](../spec/PLAN-v1.0-2026-11-06.md#5-платформа-horizon-что-известно)
и [../archive/HORIZON.md](../archive/HORIZON.md).

Статус: закрепления проверяет `tools/check_horizon_pins.py`; верно для фиксированной версии (`horizon` `66f3981`, mixins `5a64af9`,
26.3 `5f312935`, libm `0.2.16`). При смене коммита проверять заново.

## Точки внедрения

- Главная: cancellable HEAD на `horizon_core::density::graph::seeded::SeededContext::noise`
  (`graph`, `id`). Возвращает свой `Noise` для имён `litospherity:*`.
- Trait `Noise`: `range()`, `get2`, `get3`, `add_to_volume`. Переопределять `add_to_volume`
  по колонкам.
- Датапаки: поля роутера ссылаются на `minecraft:overworld/{temperature, vegetation,
  continents, erosion, ridges, depth}`. Достаточно переопределить 5 функций плотности.
- Лава: аквиферы (`aquifers` в `noise_settings`); глобально `y < min(−54, sea_level)`.
- Точка №4 `GlobalFluidPicker` — const fn `new`, создаётся один раз в `Pipeline<R>::new`;
  уровень — константа `sea_level`, без сида и без x/z; шум `surface_level` у аквифера различает только
  «сухой/затопленный», затопленные ячейки всегда получают `global.level` (море). См. Д-087.
  HEAD-хук на `<GlobalFluidPicker as FluidPicker>::compute` в пине невозможен (Д-088:
  деструктурированный параметр — E_INVALID_BINDING).

## Сборка мода

- Крейт мода: только крейты из `Cargo.lock` Horizon (serde, serde_json, toml, glam,
  libm, png). Без rayon, image, noise, без path и git-зависимостей.
- Лаунчер собирает Rust 1.99.1, Windows x86_64-pc-windows-gnu. Первая сборка 20–30 мин.
- Smart App Control на Windows ломает сборку (os error 4551).
- В CI собирается настоящий `.hnmod` (`tools/package_mod.py`). Для игры брать артефакт
  последнего зелёного CI на `main`.

## Вода

- Вода выше уровня моря: на пине механизма нет (Д-087, Д-088). Прежний вариант B (v1 без R2) из
  [../archive/T142-DECISION.md](../archive/T142-DECISION.md) отменён решением автора Д-079
  («R2 в объёме v1»); **Д-088 (2026-11-14) это уточнил: R2 вынесен из v1 на последний шаг
  плана** — релиз v1 его не ждёт (см. «Решение по R2» ниже). Технический разбор спайка
  в архиве остаётся верным.
- Холодные озёра ждут этого механизма (Д-070).

### Разведка T142-бис: миксин-хук на пине невозможен (Д-088)

Попытка повесить cancellable HEAD на вердикт воды `<GlobalFluidPicker as FluidPicker>::compute`
отклонена CI (run 43587773951, маркер удалён, после уборки CI 43589348842 — 7/7):

- id трейт-impl метода — `crate::module::<Type as Trait>::function`; плэйсхолдером без
  `<… as …>` не резолвится (E_TARGET_NOT_FOUND в пинном `resolve()`);
- пинный horizon-mixins `5a64af9` безусловно отклоняет деструктурированный параметр
  `compute(&self, [_, y, _]: [i32; 3])` (E_INVALID_BINDING, `parameters()`), до любых
  биндингов; смёрженный 2026-11-09 [#24](https://github.com/horizon-rust/horizon-mixins/pull/24)
  эту ошибку снимает для именованных частей паттерна (`y`), но не для `position`, и в пин
  не входит (пин 2026-11-05, отставание на 3 коммита от main mixins);
- типово-generic кандидаты (`NoiseBasedAquifer::compute_substance`,
  `DisabledAquifer::compute_substance`, `fill_noise`) идут в E_UNSUPPORTED_SIGNATURE
  (`validate_target`); вопрос — открытая [horizon-mixins #16](https://github.com/horizon-rust/horizon-mixins/issues/16);
- `const fn` (`FluidStatus::at`, `GlobalFluidPicker::new`) — тоже E_UNSUPPORTED_SIGNATURE.

Итог: на пине 66f3981 / mixins 5a64af9 хук на любой воде-решатель Horizon невозможен;
тестовый маркер удалён.

### Решение по R2 (Д-088)

Сначала доделываем остальные шаги плана (LOD T79, T113, T123, T77, метрики, игровые тесты,
выпуск v1). По завершении — смотрим, принял ли Horizon именованный параметр `position` в
`<GlobalFluidPicker as FluidPicker>::compute`: открытый issue
[horizon-rust/horizon#16388](https://github.com/horizon-rust/horizon/issues/16388)
(поведение не меняется; пин также должен включить horizon-mixins ≥ #24). Принял — обновить
пин и делать R2 cancellable HEAD-хуком с биндингом `position: [i32; 3]`; нет — форк Horizon.
Строка приёмки «озёр на 100 L²» (10.3) уходит вместе с R2 и v1 не блокирует.

## Ограничения Horizon

- Mod-API: нативный мод — mixin-бандл, правящий исходники перед сборкой
  (HEAD, BEFORE_CALL, RETURN, MODIFY_ARG, OVERWRITE).
- Датапаки с worldgen работают; `world_preset` не читается; новые биомы отбрасываются.
- Спавн ищется в радиусе 2048 от (0, 0).

Связанные: [relief.md](relief.md), [modes-and-versions.md](modes-and-versions.md).
