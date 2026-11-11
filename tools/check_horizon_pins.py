#!/usr/bin/env python3
"""Enforce the single Horizon / horizon-mixins / 26.3 resource pins.

The JSON file is the source of truth. CI, the client manifest, xtask, and the
real-Horizon checks all repeat these values; this script fails if any copy
drifts or if a checkout step stops using the workflow env variables.
"""
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PINS = json.loads((ROOT / "tools/horizon-revisions.json").read_text())
errors: list[str] = []


def check(what: str, actual: str, expected: str) -> None:
    if actual != expected:
        errors.append(f"{what}: {actual!r} != {expected!r}")


native = json.loads((ROOT / "mod/litospherity/horizon-native.json").read_text())
check("horizon-native.json game", native["game"], PINS["game"])
check("horizon-native.json horizon", native["horizon"], PINS["horizon"])
check("horizon-native.json mixins", native["mixins"], PINS["horizon_mixins"])

client = tomllib.loads(
    (ROOT / "mod/litospherity/mixins/litospherity/horizon-mixin.toml").read_text()
)
check(
    "client horizon_revision",
    client["compatibility"]["horizon_revision"],
    PINS["horizon"],
)

xtask = (ROOT / "tools/xtask/src/main.rs").read_text()
match = re.search(r'const HORIZON_REV: &str = "([0-9a-f]{40})";', xtask)
if not match:
    errors.append("xtask: не найден const HORIZON_REV")
else:
    check("xtask HORIZON_REV", match.group(1), PINS["horizon"])

ci = (ROOT / ".github/workflows/ci.yml").read_text()
for key, pin in (
    ("HORIZON_REV", PINS["horizon"]),
    ("HORIZON_MIXINS_REV", PINS["horizon_mixins"]),
    ("MCMETA_REV", PINS["mcmeta"]),
    ("LIBM_TAG", PINS["libm_tag"]),
):
    match = re.search(rf"^\s*{key}:\s*['\"]?([^'\"\s#]+)['\"]?\s*$", ci, re.M)
    if not match:
        errors.append(f"ci.yml: нет env.{key}")
    else:
        check(f"ci.yml env.{key}", match.group(1), pin)

# Every external checkout must use the env expression, never a second literal.
repository = None
for raw in ci.splitlines():
    line = raw.strip()
    if line.startswith("repository:"):
        repository = line.split(":", 1)[1].strip().strip("'\"")
    elif line.startswith("ref:"):
        ref = line.split(":", 1)[1].strip().strip("'\"")
        expected = {
            "horizon-rust/horizon": "${{ env.HORIZON_REV }}",
            "horizon-rust/horizon-mixins": "${{ env.HORIZON_MIXINS_REV }}",
            "misode/mcmeta": "${{ env.MCMETA_REV }}",
            "rust-lang/compiler-builtins": "${{ env.LIBM_TAG }}",
        }.get(repository or "")
        if expected is None:
            errors.append(f"ci.yml: ref {ref!r} у неизвестного repository={repository!r}")
        else:
            check(f"ci.yml checkout {repository} ref", ref, expected)

for stale in (
    "f319701e8c9ab657ff562250b3ba07b2d654e3bc",
    "0d6b103c70633d8fd9443d8d0acd3aadaacedb06",
    "e8cd0aa38cffef49cb2e088ef716baa107148c02",
):
    if stale in ci or stale in xtask or stale in json.dumps(native):
        errors.append(f"осталось старое закрепление {stale}")

run_sh = (ROOT / "tools/horizon-core-check/run.sh").read_text()
if "horizon-revisions.json" not in run_sh:
    errors.append("horizon-core-check/run.sh не читает единый файл закреплений")
package = (ROOT / "tools/package_mod.py").read_text()
if "horizon-revisions.json" not in package:
    errors.append("package_mod.py не читает единый файл закреплений")

if errors:
    for error in errors:
        print(f"PIN ERROR: {error}", file=sys.stderr)
    raise SystemExit(1)
print(
    "Horizon pins OK: "
    f"{PINS['game']} / {PINS['horizon']} / {PINS['horizon_mixins']} / {PINS['mcmeta']}"
)
