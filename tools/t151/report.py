#!/usr/bin/env python3
"""T151: сводка `stats --json` по сидам против целей 10.1 (рельеф и границы).

    python3 tools/t151/report.py <каталог прогонов> [--out <каталог>] [--request <json>] [--sha <коммит>]

Каталог прогонов: `<режим>_<сид>.json` (вывод stats --json), рядом могут лежать
`.exit` (код возврата), `.time` (секунды) и `.err` (stderr). С `--out` пишет
`results.json` (все прогоны одним файлом) и `summary.md`; сводка печатается в
stdout в любом случае. Скрипт ничего не решает о приёмке: он только считает,
сколько сидов попало в цель по каждой метрике.
"""
import glob
import json
import os
import statistics
import sys

# метрика: (gameplay lo, hi), (scientific lo, hi) — PLAN-REVISION-2026-11-08, 10.1
TARGETS = {
    "затопленная суша, %": ((None, 5), (None, 5)),
    "y 63–70, %": ((20, 35), (30, 45)),
    "y 70–90, %": ((30, 45), (25, 40)),
    "y 90–130, %": ((12, 25), (10, 20)),
    "y > 130, %": ((3, 10), (3, 10)),
    "E4, %": ((30, 50), (45, 60)),
    "E3+E2, %": ((25, 40), (20, 30)),
    "E0+E1, %": ((5, 15), (5, 15)),
    "E5+E6, %": ((3, 10), (3, 10)),
    "долина, %": ((None, 8), (None, 6)),
    "перепад в окне 128, медиана": ((10, None), (6, None)),
    "плоские окна, %": ((None, 15), (None, 25)),
    "вода в русле, %": ((95, None), (95, None)),
    "скачок влажности, климатическая суша, %": ((None, 2), (None, 2)),
    "скачок влажности, вся суша, %": ((None, 2), (None, 2)),
    "ветер, градусы": ((None, 5), (None, 5)),
    "прямые границы, %": ((None, 1), (None, 1)),
    "тонкие полоски, %": ((None, 0.05), (None, 0.05)),
}
MODES = ("gameplay", "scientific")


def metrics(j):
    h, e, b = j["hyps_pct"], j["erosion_pct"], j["boundary"]
    return {
        "затопленная суша, %": j["flooded_land_pct"],
        "y 63–70, %": h[1],
        "y 70–90, %": h[2],
        "y 90–130, %": h[3],
        "y > 130, %": h[4],
        "E4, %": e[4],
        "E3+E2, %": e[3] + e[2],
        "E0+E1, %": e[0] + e[1],
        "E5+E6, %": e[5] + e[6],
        "долина, %": j["valley_pct"],
        "перепад в окне 128, медиана": j["relief_median"],
        "плоские окна, %": j["flat_windows_pct"],
        "вода в русле, %": j["river_water_pct"],
        "скачок влажности, климатическая суша, %": b["climate_humidity_jump_pct"],
        "скачок влажности, вся суша, %": b["humidity_jump_pct"],
        "ветер, градусы": b["wind_max_deg"],
        "прямые границы, %": b["straight_pct"],
        "тонкие полоски, %": b["thin_pct"],
    }


def goal(lo, hi):
    if lo is not None and hi is not None:
        return f"{lo}–{hi}"
    return f"≤ {hi}" if lo is None else f"≥ {lo}"


def in_goal(x, lo, hi):
    return x is not None and (lo is None or x >= lo) and (hi is None or x <= hi)


def read_text(path, default=""):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return default


def load(directory):
    runs, failures = [], []
    stems = set()
    for ext in ("json", "exit", "err"):
        for f in glob.glob(os.path.join(directory, "**", f"*.{ext}"), recursive=True):
            stem = f[: -len(ext) - 1]
            if os.path.basename(stem).split("_")[0] in MODES:
                stems.add(stem)
    for stem in sorted(stems):
        mode, seed = os.path.basename(stem).split("_", 1)
        code = read_text(stem + ".exit", "?").strip()
        seconds = read_text(stem + ".time").strip()
        try:
            with open(stem + ".json", encoding="utf-8") as f:
                data = json.load(f)
            metrics(data)
        except (OSError, ValueError, KeyError, TypeError, IndexError) as ex:
            err = [l for l in read_text(stem + ".err").splitlines() if "сеть рек №" not in l]
            failures.append(
                {"mode": mode, "seed": seed, "exit": code, "seconds": seconds,
                 "error": str(ex), "stderr_tail": err[-12:]}
            )
            continue
        data["_seconds"] = float(seconds) if seconds else None
        runs.append(data)
    return runs, failures


def summary(runs, failures, request, sha):
    out = []
    name = request.get("name", "?")
    out.append(f"# T151: замер «{name}»")
    out.append("")
    if request.get("note"):
        out.append(request["note"])
        out.append("")
    out.append(f"- Коммит: `{sha or '?'}`")
    out.append(f"- Версия генератора: `{request.get('version', '?')}`")
    out.append(f"- Флаги `stats`: `{request.get('args', '?')}`")
    out.append(f"- Прогонов готово: {len(runs)}, упало: {len(failures)}")
    volume = sum(1 for r in runs if r["boundary"].get("acceptance_volume"))
    out.append(f"- Прогонов в приёмочном объёме границ (10.1): {volume} из {len(runs)}")
    out.append("")
    for mi, mode in enumerate(MODES):
        rows = [(r["seed"], metrics(r), r) for r in runs if r["mode"] == mode]
        out.append(f"## {mode}: {len(rows)} сидов")
        out.append("")
        if not rows:
            out.append("Нет готовых прогонов.")
            out.append("")
            continue
        out.append("| Метрика | Цель | Мин | Медиана | Макс | В цели | Сиды вне цели |")
        out.append("|---|---|---|---|---|---|---|")
        for key, target in TARGETS.items():
            lo, hi = target[mi]
            values = [(seed, m[key]) for seed, m, _ in rows]
            known = [v for _, v in values if v is not None]
            bad = [(s, v) for s, v in values if not in_goal(v, lo, hi)]
            bad_text = ", ".join(
                f"{s} ({'нет' if v is None else format(v, '.2f')})" for s, v in bad
            )
            if known:
                stats = f"{min(known):.2f} | {statistics.median(known):.2f} | {max(known):.2f}"
            else:
                stats = "— | — | —"
            out.append(
                f"| {key} | {goal(lo, hi)} | {stats} | {len(values) - len(bad)}/{len(values)} | {bad_text} |"
            )
        out.append("")
        windows = [r["relief_windows"] for _, _, r in rows]
        land = [r["boundary"]["land"] for _, _, r in rows]
        times = [r["_seconds"] for _, _, r in rows if r.get("_seconds") is not None]
        out.append(
            f"Объём: окон рельефа на сид {min(windows)}…{max(windows)}, "
            f"точек суши в сетке границ {min(land)}…{max(land)}."
            + (f" Время прогона {min(times):.0f}…{max(times):.0f} с." if times else "")
        )
        out.append("")
    if failures:
        out.append("## Упавшие прогоны")
        out.append("")
        for f in failures:
            out.append(f"### {f['mode']} {f['seed']}: код {f['exit']}, {f['seconds'] or '?'} с")
            out.append("")
            out.append(f"Разбор результата: {f['error']}")
            out.append("")
            out.append("```")
            out.extend(f["stderr_tail"] or ["(stderr пуст)"])
            out.append("```")
            out.append("")
    return "\n".join(out) + "\n"


def main(argv):
    if not argv or argv[0].startswith("--"):
        print(__doc__)
        return 2
    directory = argv[0]
    opts = dict(zip(argv[1::2], argv[2::2]))
    request = {}
    if "--request" in opts:
        with open(opts["--request"], encoding="utf-8") as f:
            request = json.load(f)
    runs, failures = load(directory)
    text = summary(runs, failures, request, opts.get("--sha"))
    sys.stdout.write(text)
    if "--out" in opts:
        os.makedirs(opts["--out"], exist_ok=True)
        with open(os.path.join(opts["--out"], "summary.md"), "w", encoding="utf-8") as f:
            f.write(text)
        with open(os.path.join(opts["--out"], "results.json"), "w", encoding="utf-8") as f:
            json.dump(
                {"request": request, "sha": opts.get("--sha"), "runs": runs, "failures": failures},
                f, ensure_ascii=False, indent=1, sort_keys=True,
            )
            f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
