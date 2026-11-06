#!/usr/bin/env python3
"""Один раз генерирует таблицы констант для litho-core/src/noise_tables.rs.

Скрипт нужен только разработчику, НЕ вызывается при сборке: в крейте нет
build.rs и нет внешних зависимостей (план 5.4, 7.2). В сгенерированном файле
лежат готовые числа, поэтому во время работы генератора тригонометрия не
вызывается и результат одинаков на Linux, Windows msvc и Windows gnu.

Запуск из корня репозитория:  python3 tools/gen_tables.py
"""

from __future__ import annotations

import math
import pathlib

MASK = (1 << 64) - 1
GOLDEN = 0x9E3779B97F4A7C15
OCTAVES = 8  # максимум октав, который умеет fbm2

OUT = pathlib.Path(__file__).resolve().parents[1] / "crates/litho-core/src/noise_tables.rs"


def splitmix64(z: int) -> int:
    z = (z + GOLDEN) & MASK
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
    return z ^ (z >> 31)


def unit(h: int) -> float:
    return (h >> 11) * (1.0 / 9007199254740992.0)


def main() -> None:
    lines = [
        "// СГЕНЕРИРОВАНО tools/gen_tables.py — не править руками.",
        "// 256 единичных градиентов для шума Перлина (план 7.2) и повороты",
        "// октав fBm на углы 0.61*k радиан. Все числа — константы,",
        "// тригонометрия во время работы не вызывается.",
        "",
        "pub const DIRS: [[f64; 2]; 256] = [",
    ]
    for k in range(256):
        angle = 2.0 * math.pi * unit(splitmix64(0x1234_5678_9ABC_DEF0 + k))
        x, z = math.cos(angle), math.sin(angle)
        lines.append(f"    [{x!r}, {z!r}],")
    lines.append("];")
    lines.append("")
    lines.append("pub const ROT: [[f64; 2]; OCTAVES] = [")
    for k in range(OCTAVES):
        angle = 0.61 * k
        lines.append(f"    [{math.cos(angle)!r}, {math.sin(angle)!r}], // k = {k}")
    lines.append("];")
    lines.append("")
    lines.append("pub const OCTAVES: usize = 8;")
    lines.append("")
    OUT.write_text("\n".join(lines), encoding="utf-8")
    print(f"записано {OUT.relative_to(pathlib.Path(__file__).resolve().parents[1])}")


if __name__ == "__main__":
    main()
