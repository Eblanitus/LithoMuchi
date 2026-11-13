#!/usr/bin/env python3
"""T151: сводка `stats --json` по сидам против целей 10.1 (рельеф и границы).

    python3 tools/t151/report.py <каталог прогонов> [--out <каталог>] [--request <json>] [--sha <коммит>]

Каталог прогонов: `<режим>_<сид>.json` (вывод stats --json), рядом могут лежать
`.exit` (код возврата), `.time` (секунды) и `.err` (stderr). Варианты подбора
(`variants` в запросе, флаг `--set`) лежат в подкаталогах `<вариант>/`.
С `--out` пишет `summary.md` и, если в запросе нет `"results": "summary"`,
`results.json` (все прогоны одним файлом); сводка печатается в stdout в любом
случае. Скрипт ничего не решает о приёмке: он только считает, сколько сидов
попало в цель по каждой метрике.
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
    "скачок влажности, поле H, вся суша, %": ((None, 2), (None, 2)),
    "скачок влажности, метки биомов, вся суша, %": ((None, 2), (None, 2)),
    "ветер, градусы": ((None, 5), (None, 5)),
    "прямые границы, %": ((None, 1), (None, 1)),
    "тонкие полоски, %": ((None, 0.05), (None, 0.05)),
    # 10.1 «Реки и озёра» и «климат влияет на форму» — считаются начиная с
    # фиксации 2026-11-12; в старых прогонах ключа `plan` нет, строки пропустятся.
    "климат влияет на форму": ((True, True), (True, True)),
    "вода озёр на суше, %": ((None, None), (None, None)),
    "озёр на 100 L² суши": ((1.0, None), (1.0, None)),
    "холодные зоны: котловины без воды, %": ((None, None), (None, None)),
    "холодные зоны: сухие котловины, %": ((None, None), (None, None)),
    # 12.4 плана: ориентиры, а не законы — если картинка хороша, а метрика
    # нет, правится диапазон с записью в журнал. Цели едины для режимов.
    # Ключа `climate124` нет в прогонах до 2026-11-13 — строки пропустятся.
    "суша 12.4, %": ((25, 40), (25, 40)),
    "горы 12.4, %": ((5, 20), (5, 20)),
    "пустыни 12.4, %": ((15, 30), (15, 30)),
    "леса 12.4, %": ((30, 50), (30, 50)),
    "полярные 12.4, %": ((10, 25), (10, 25)),
    "точка появления 12.4": ((True, True), (True, True)),
    # Д-074: профиль P(φ) по 18 поясам. Ключа нет в climate124-1 и старше,
    # поэтому в старых сводках строка пропускается, а не «0/16».
    "зональный профиль осадков": ((True, True), (True, True)),
}
MODES = ("gameplay", "scientific")
CAUSE_NAMES = {
    "climate": "климатическая суша",
    "river": "русло и его кромка",
    "gallery": "галерейный лес",
    "swamp": "болото",
    "dry_basin": "сухая котловина",
    "estuary": "эстуарий",
    "other": "прочее",
}


def metrics(j):
    h, e, b = j["hyps_pct"], j["erosion_pct"], j["boundary"]
    # Все ключи TARGETS есть всегда (неизвестное — None): иначе сводка
    # падает с KeyError на прогонах, где нет `plan` или `climate124`.
    out = {
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
        # Поле H — число, которое уходит в игру. Метка биома у русла, болота
        # и котловины несёт служебный класс влажности, а не климатический.
        "скачок влажности, поле H, вся суша, %": b.get("field_humidity_jump_pct"),
        "скачок влажности, метки биомов, вся суша, %": b["humidity_jump_pct"],
        "ветер, градусы": b["wind_max_deg"],
        "прямые границы, %": b["straight_pct"],
        "тонкие полоски, %": b["thin_pct"],
        **_plan_metrics(j),
        **_climate124_metrics(j),
    }
    for key in TARGETS:
        out.setdefault(key, None)
    return out


def _climate124_metrics(j):
    """12.4: доли климата и точка появления из ключа `climate124`.

    В прогонах до 2026-11-13 ключа нет — тогда строки не выводятся вовсе,
    как и у `plan`: старые сводки остаются читаемыми.
    """
    c = j.get("climate124")
    if not isinstance(c, dict):
        return {}
    out = {
        "суша 12.4, %": c.get("land_pct"),
        "горы 12.4, %": c.get("mountains_pct"),
        "пустыни 12.4, %": c.get("deserts_pct"),
        "леса 12.4, %": c.get("forests_pct"),
        "полярные 12.4, %": c.get("polar_pct"),
        "точка появления 12.4": c.get("spawn_good"),
    }
    # Д-074: в старых прогонах вложенного ключа нет. Строку не добавляем
    # вовсе, чтобы mode_table пропустил её как «метрики нет в этих прогонах».
    z = c.get("zonal_profile")
    if isinstance(z, dict):
        out["зональный профиль осадков"] = z.get("in_target")
    return out


def _plan_metrics(j):
    """10.1: «климат влияет на форму» и вода озёр из нового ключа `plan`.

    В прогонах до 2026-11-12 ключа нет — тогда строки не выводятся вовсе,
    чтобы старые сводки оставались читаемыми.
    """
    plan = j.get("plan")
    if not isinstance(plan, dict):
        return {}
    shape = plan.get("climate_shape") or {}
    rivers = plan.get("rivers_lakes") or {}
    zones = {z.get("name"): z for z in shape.get("zones") or []}
    # Д-069: провал «озёр в холодных зонах» — это в основном долг R2, а не
    # свойство мира: в полярной зоне 5 % суши приходится на котловины, вода
    # которых ждёт T143/T145. Отдельная строка не даёт читать «озёр 0 %» как
    # «озёр тут не бывает».
    cold_deferred = max(
        (zones.get(n) or {}).get("deferred_pct") or 0.0 for n in ("polar", "boreal")
    )
    cold_dry = max((zones.get(n) or {}).get("dry_pct") or 0.0 for n in ("polar", "boreal"))
    return {
        "климат влияет на форму": shape.get("pass"),
        "вода озёр на суше, %": rivers.get("lake_pct"),
        "озёр на 100 L² суши": rivers.get("lakes_per_100_l2"),
        "холодные зоны: котловины без воды, %": cold_deferred,
        "холодные зоны: сухие котловины, %": cold_dry,
    }


def goal(lo, hi):
    if lo is True and hi is True:
        return "в цели"
    if lo is not None and hi is not None:
        return f"{lo}–{hi}"
    if lo is None and hi is None:
        return "справка"
    return f"≤ {hi}" if lo is None else f"≥ {lo}"


def in_goal(x, lo, hi):
    if lo is True and hi is True:
        return x is True
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
        variant = os.path.relpath(os.path.dirname(stem), directory)
        variant = "" if variant == "." else variant
        code = read_text(stem + ".exit", "?").strip()
        seconds = read_text(stem + ".time").strip()
        try:
            with open(stem + ".json", encoding="utf-8") as f:
                data = json.load(f)
            metrics(data)
        except (OSError, ValueError, KeyError, TypeError, IndexError) as ex:
            err = [l for l in read_text(stem + ".err").splitlines() if "сеть рек №" not in l]
            failures.append(
                {"variant": variant, "mode": mode, "seed": seed, "exit": code, "seconds": seconds,
                 "error": str(ex), "stderr_tail": err[-12:]}
            )
            continue
        data["_seconds"] = float(seconds) if seconds else None
        data["_variant"] = variant
        runs.append(data)
    return runs, failures


def mode_table(rows, mi, keys):
    out = []
    out.append("| Метрика | Цель | Мин | Медиана | Макс | В цели | Сиды вне цели |")
    out.append("|---|---|---|---|---|---|---|")
    for key in keys:
        lo, hi = TARGETS[key][mi]
        values = [(seed, m[key]) for seed, m, _ in rows]
        known = [v for _, v in values if v is not None]
        if not known:
            continue  # метрики нет в этих прогонах (старый формат)
        bad = [(s, v) for s, v in values if not in_goal(v, lo, hi)]
        bad_text = ", ".join(
            f"{s} ({'нет' if v is None else ('да' if v is True else 'нет' if v is False else format(v, '.2f'))})"
            for s, v in bad
        )
        if known and isinstance(known[0], bool):
            good = sum(1 for v in known if v)
            stats = f"да {good}/{len(known)} | — | —"
        elif known:
            stats = f"{min(known):.2f} | {statistics.median(known):.2f} | {max(known):.2f}"
        else:
            stats = "— | — | —"
        out.append(
            f"| {key} | {goal(lo, hi)} | {stats} | {len(values) - len(bad)}/{len(values)} | {bad_text} |"
        )
    out.append("")
    return out


def compare_table(runs, variants, mi, mode, keys):
    """Варианты подбора рядом: мин / медиана / макс и число сидов в цели."""
    names = [v for v in variants if any(r["_variant"] == v and r["mode"] == mode for r in runs)]
    if len(names) < 2:
        return []
    out = [f"### {mode}: варианты рядом (мин / медиана / макс, в цели)", ""]
    out.append("| Метрика | Цель | " + " | ".join(f"`{n or 'без имени'}`" for n in names) + " |")
    out.append("|---|---|" + "---|" * len(names))
    for key in keys:
        lo, hi = TARGETS[key][mi]
        if all(metrics(r)[key] is None for r in runs if r["mode"] == mode):
            continue
        cells = []
        for name in names:
            values = [metrics(r)[key] for r in runs if r["_variant"] == name and r["mode"] == mode]
            known = [v for v in values if v is not None]
            ok = sum(1 for v in values if in_goal(v, lo, hi))
            if known and isinstance(known[0], bool):
                cells.append(f"да {sum(1 for v in known if v)}/{len(values)}")
            elif known:
                cells.append(
                    f"{min(known):.2f} / {statistics.median(known):.2f} / {max(known):.2f}, {ok}/{len(values)}"
                )
            else:
                cells.append("—")
        out.append(f"| {key} | {goal(lo, hi)} | " + " | ".join(cells) + " |")
    out.append("")
    return out


def summary(runs, failures, request, sha):
    out = []
    name = request.get("name", "?")
    keys = [k for k in TARGETS if k in request.get("rows", TARGETS)]
    declared = {v["name"]: v for v in request.get("variants", [])}
    variants = list(declared) + sorted({r["_variant"] for r in runs} - set(declared))
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
    if any(r.get("set") for r in runs):
        out.append(
            "- Варианты с `--set` считаются на изменённых параметрах: это подбор, не приёмка."
        )
    if len(keys) < len(TARGETS):
        out.append("- Показаны не все метрики: остальные на этом объёме замера не считаются.")
    out.append("")
    if len(variants) > 1:
        out.append("## Варианты")
        out.append("")
        out.append("| Вариант | Режимы | `--set` | Хеши параметров |")
        out.append("|---|---|---|---|")
        for v in variants:
            mine = [r for r in runs if r["_variant"] == v]
            modes = ", ".join(sorted({r["mode"] for r in mine}))
            hashes = ", ".join(sorted({r.get("params_hash", "?") for r in mine}))
            sets = declared.get(v, {}).get("set", "") or "—"
            out.append(f"| `{v}` | {modes} | `{sets}` | `{hashes}` |")
        out.append("")
        for mi, mode in enumerate(MODES):
            out.extend(compare_table(runs, variants, mi, mode, keys))
    for variant in variants:
        prefix = f"`{variant}`, " if variant else ""
        for mi, mode in enumerate(MODES):
            rows = [
                (r["seed"], metrics(r), r)
                for r in runs
                if r["mode"] == mode and r["_variant"] == variant
            ]
            if not rows:
                if len(variants) == 1:
                    out.append(f"## {mode}: нет готовых прогонов")
                    out.append("")
                continue
            out.append(f"## {prefix}{mode}: {len(rows)} сидов")
            out.append("")
            out.extend(mode_table(rows, mi, keys))
            windows = [r["relief_windows"] for _, _, r in rows]
            land = [r["boundary"]["land"] for _, _, r in rows]
            samples = [r["land_samples"] for _, _, r in rows]
            times = [r["_seconds"] for _, _, r in rows if r.get("_seconds") is not None]
            causes = {}
            for _, _, r in rows:
                for cause, count in r["boundary"].get("humidity_jump_cause_pairs", {}).items():
                    causes[cause] = causes.get(cause, 0) + count
            total = sum(causes.values())
            if total:
                parts = ", ".join(
                    f"{CAUSE_NAMES.get(c, c)} {100 * n / total:.1f} %"
                    for c, n in sorted(causes.items(), key=lambda kv: -kv[1])
                    if n
                )
                out.append(f"Скачки по меткам биомов, {total} пар по всем сидам: {parts}.")
                out.append("")
            out.append(
                f"Объём: точек суши в основной сетке {min(samples)}…{max(samples)}, "
                f"окон рельефа на сид {min(windows)}…{max(windows)}, "
                f"точек суши в сетке границ {min(land)}…{max(land)}."
                + (f" Время прогона {min(times):.0f}…{max(times):.0f} с." if times else "")
            )
            out.append("")
    if failures:
        out.append("## Упавшие прогоны")
        out.append("")
        for f in failures:
            label = f"{f['variant']} " if f.get("variant") else ""
            out.append(f"### {label}{f['mode']} {f['seed']}: код {f['exit']}, {f['seconds'] or '?'} с")
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
        with open(os.path.join(opts["--out"], "status.json"), "w", encoding="utf-8") as f:
            json.dump({"runs": len(runs), "failures": len(failures)}, f)
            f.write("\n")
        if request.get("results", "full") != "summary":
            with open(os.path.join(opts["--out"], "results.json"), "w", encoding="utf-8") as f:
                json.dump(
                    {"request": request, "sha": opts.get("--sha"), "runs": runs, "failures": failures},
                    f, ensure_ascii=False, indent=1, sort_keys=True,
                )
                f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
