# Вики Litospherity

Справочник «как устроен мир сейчас». Пишется по принципу: здесь только то, что верно
сегодня. История (какой была формула раньше и почему её поменяли) — в
[DECISIONS.md](../DECISIONS.md) и в [../archive/](../archive/). Если в статье сказано
«Д-0xx», значит решение изменило формулу: итоговая формула указана в статье, сверена с
кодом (`crates/litho-core/src/`, параметры в `params.rs`).

План работ — в [../PLAN.md](../PLAN.md).

| № | Тема | Файл |
|---|---|---|
| 1 | Замысел и решения автора | [intent.md](intent.md) |
| 2 | Плиты и кора | [plates-and-crust.md](plates-and-crust.md) |
| 3 | Климат: полюса, ветер, влага | [climate.md](climate.md) |
| 4 | Биомы и ванильные параметры | [biomes.md](biomes.md) |
| 5 | Реки и озёра | [rivers-and-lakes.md](rivers-and-lakes.md) |
| 6 | Рельеф и своя высота | [relief.md](relief.md) |
| 7 | Horizon: точки внедрения, сборка, вода | [horizon.md](horizon.md) |
| 8 | Режимы и версии | [modes-and-versions.md](modes-and-versions.md) |
| 9 | Скорость и LOD | [speed-and-lod.md](speed-and-lod.md) |
| 10 | Метрики и приёмка | [metrics-and-acceptance.md](metrics-and-acceptance.md) |
| 11 | Глоссарий | [glossary.md](glossary.md) |

Статус статей: **верно** — сверено с кодом; **по плану** — формула из плана v1.0,
в коде не проверена; **ошибка** — помечено в тексте, исправление указано рядом.
