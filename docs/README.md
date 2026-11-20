# Документация Litospherity

Начинать с [PLAN.md](PLAN.md): что готово, что осталось до v1 и в каком порядке.

| Что | Где |
|---|---|
| План до v1 (состояние, оставшиеся шаги, правила) | [PLAN.md](PLAN.md) |
| Переход на Fabric (Ф0–Ф7) | [PLAN-FABRIC.md](PLAN-FABRIC.md) |
| Вода выше уровня моря на Fabric, `v1-dev-r2` (В1–В5 сделаны; заменён водным обновлением, В6–В7 отменены) | [PLAN-WATER-FABRIC.md](PLAN-WATER-FABRIC.md) |
| **Действующий план воды:** типы водоёмов, вода выше моря от `S0` (ВО0–ВО9, принят автором, Д-129; формулы — раздел 6, этапы — раздел 9) | [PLAN-WATER-UPDATE.md](PLAN-WATER-UPDATE.md) |
| Перенос на новый Horizon (.hmod) и воды из Java в Horizon (Г0–Г8) | [PLAN-HORIZON-PORT.md](PLAN-HORIZON-PORT.md) |
| Сверка кода 26.3 и выбранная точка внедрения (Ф6-В0) | [FABRIC-WATER-NOTES.md](FABRIC-WATER-NOTES.md) |
| Спецификация: формулы и критерии v1 (план v1.0 и пересмотр v1.1); формулы воды — в [PLAN-WATER-UPDATE.md](PLAN-WATER-UPDATE.md) раздел 6 | [spec/](spec/) |
| Справочник: как устроен мир сейчас, по темам | [wiki/](wiki/README.md) |
| Журнал решений Д-001… (только дописывается) | [DECISIONS.md](DECISIONS.md) |
| Параметры `v0` (заморожена) и `v1-dev` | [PARAMS.md](PARAMS.md), [PARAMS-V1-DEV.md](PARAMS-V1-DEV.md) |
| Замеры T151 | `t151/` |
| Замеры водного обновления ВО0 (`water-stats`, `s0-node-cost`) | [probe/water-update-vo0/probe.txt](probe/water-update-vo0/probe.txt) |
| Картинки | `images/` |
| История: старые статусы, отчёты, разведки | [archive/](archive/) |

Документы в `archive/` не руководство: они описывают прошлое на свою дату. В `spec/` действуют
формулы и критерии, а статусы и порядок работ оттуда заменены [PLAN.md](PLAN.md).
