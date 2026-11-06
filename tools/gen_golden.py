#!/usr/bin/env python3
"""Генерирует golden/v0/primitives.txt — эталоны примитивов (план 14.3).

Эталон — это биты результата (`u64` или биты `f64`), а не десятичная запись:
CI сравнивает их побитово на Linux, Windows msvc и Windows gnu. Скрипт
повторяет формулы из crates/litho-core/src/{hash,math}.rs на обычном Python
float (= IEEE 754 binary64), поэтому расхождение сразу видно.

Запуск: python3 tools/gen_golden.py
Обновлять эталон v0 можно; эталоны выпущенных версий — нельзя (6.8).
"""

from __future__ import annotations

import math
import pathlib
import struct

MASK = (1 << 64) - 1
LN_2 = 0.6931471805599453
OUT = pathlib.Path(__file__).resolve().parents[1] / "golden/v0/primitives.txt"


def sm(z: int) -> int:
    z = (z + 0x9E3779B97F4A7C15) & MASK
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
    return z ^ (z >> 31)


def fnv(text: str) -> int:
    h = 0xCBF29CE484222325
    for byte in text.encode("utf-8"):
        h = ((h ^ byte) * 0x100000001B3) & MASK
    return h


def hash3(s: int, a: int, b: int, c: int) -> int:
    x0 = sm(s ^ ((a & MASK) * 0x9E3779B97F4A7C15) & MASK)
    x1 = sm(x0 ^ ((b & MASK) * 0xC2B2AE3D27D4EB4F) & MASK)
    return sm(x1 ^ ((c & MASK) * 0x165667B19E3779F9) & MASK)


def unit(h: int) -> float:
    return (h >> 11) * (1.0 / 9007199254740992.0)


def clamp(x: float, lo: float, hi: float) -> float:
    return lo if x < lo else hi if x > hi else x


def clamp01(x: float) -> float:
    return clamp(x, 0.0, 1.0)


def smoothstep(a: float, b: float, x: float) -> float:
    if a == b:
        return 0.0 if x < a else 1.0
    t = clamp01((x - a) / (b - a))
    return t * t * (3.0 - 2.0 * t)


def lerp(a: float, b: float, t: float) -> float:
    return a + (b - a) * t


def piecewise(xs: list[float], ys: list[float], x: float) -> float:
    if not xs:
        return 0.0
    if x <= xs[0]:
        return ys[0]
    for i in range(1, len(xs)):
        if x <= xs[i]:
            return lerp(ys[i - 1], ys[i], (x - xs[i - 1]) / (xs[i] - xs[i - 1]))
    return ys[-1]


def dexp(x: float) -> float:
    if math.isnan(x) or x == math.inf:
        return x
    if x == -math.inf:
        return 0.0
    x = clamp(x, -700.0, 700.0)
    q = x / LN_2
    kf = math.floor(q + 0.5) if q >= 0 else math.ceil(q - 0.5)
    k = int(kf)
    r = x - kf * LN_2
    total = 1.0
    term = 1.0
    for n in range(1, 21):
        term *= r / n
        total += term
    biased = k + 1023
    if biased <= 0 or biased >= 2047:
        return 0.0
    return math.ldexp(1.0, k) * total


def dln(x: float) -> float:
    if x <= 0.0 or not math.isfinite(x):
        return math.nan
    m = x
    shift = 0.0
    while m < 2.2250738585072014e-308:
        m *= 18014398509481984.0
        shift -= 54.0
    bits = struct.unpack(">Q", struct.pack(">d", m))[0]
    exponent = float(((bits >> 52) & 0x7FF) - 1023)
    mantissa = struct.unpack(
        ">d", struct.pack(">Q", (bits & 0x000FFFFFFFFFFFFF) | (1023 << 52))
    )[0]
    y = (mantissa - 1.0) / (mantissa + 1.0)
    y2 = y * y
    term = y
    total = 0.0
    for k in range(31):
        total += term / (2 * k + 1)
        term *= y2
    return (exponent + shift) * LN_2 + 2.0 * total


def dpow(base: float, exponent: float) -> float:
    if exponent == 0.0 or base == 1.0:
        return 1.0
    if base == 0.0:
        return 0.0 if exponent > 0.0 else math.inf
    return dexp(exponent * dln(base))


def bits_f64(x: float) -> int:
    return struct.unpack(">Q", struct.pack(">d", x))[0]


def main() -> None:
    h = hash3(42, 1, -2, 3)
    entries: list[tuple[str, int]] = [
        ("splitmix64_0", sm(0)),
        ("splitmix64_1", sm(1)),
        ("splitmix64_max", sm(MASK)),
        ("splitmix64_deadbeef", sm(0xDEADBEEF)),
        ("fnv1a64_plates.seed", fnv("plates.seed")),
        ("fnv1a64_poles.seed", fnv("poles.seed")),
        ("fnv1a64_empty", fnv("")),
        ("hash3_42_1_-2_3", h),
        ("hash3_0_0_0_0", hash3(0, 0, 0, 0)),
        (
            "hash3_deadbeef_-123456789_987654321_-1",
            hash3(0xDEADBEEF, -123456789, 987654321, -1),
        ),
        ("unit_0", bits_f64(unit(0))),
        ("unit_max", bits_f64(unit(MASK))),
        ("unit_hash3_42_1_-2_3", bits_f64(unit(h))),
        ("smoothstep_0_1_0.3", bits_f64(smoothstep(0.0, 1.0, 0.3))),
        ("smoothstep_1_0_0.3", bits_f64(smoothstep(1.0, 0.0, 0.3))),
        ("smoothstep_-1_2_0", bits_f64(smoothstep(-1.0, 2.0, 0.0))),
        ("lerp_1_3_0.25", bits_f64(lerp(1.0, 3.0, 0.25))),
        ("clamp_-2_0_1", bits_f64(clamp(-2.0, 0.0, 1.0))),
        ("piecewise_1.5", bits_f64(piecewise([0.0, 1.0, 2.0], [10.0, 20.0, 40.0], 1.5))),
        ("dexp_0", bits_f64(dexp(0.0))),
        ("dexp_1", bits_f64(dexp(1.0))),
        ("dexp_-3.5", bits_f64(dexp(-3.5))),
        ("dexp_12", bits_f64(dexp(12.0))),
        ("dln_1", bits_f64(dln(1.0))),
        ("dln_2", bits_f64(dln(2.0))),
        ("dln_0.5", bits_f64(dln(0.5))),
        ("dln_10", bits_f64(dln(10.0))),
        ("dpow_0.5_1.6", bits_f64(dpow(0.5, 1.6))),
        ("dpow_2_0.5", bits_f64(dpow(2.0, 0.5))),
    ]
    lines = [
        "# golden/v0: эталоны примитивов litho-core (план 14.3).",
        "# Формат: <имя> <16 шестнадцатеричных цифр>.",
        "# Для u64 это само значение, для f64 — биты значения (to_bits).",
        "# Сгенерировано tools/gen_golden.py; для v0 обновлять разрешено.",
    ]
    lines.extend(f"{name} {value:016x}" for name, value in entries)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"записано {OUT.relative_to(pathlib.Path(__file__).resolve().parents[1])}: {len(entries)} значений")


if __name__ == "__main__":
    main()
