#!/usr/bin/env python3
"""Package xtask's worldgen bundle together with the client hooks (stdlib only).

This is the single source of a real ``litospherity.hnmod``: client hooks, the
injected worldgen manifest, the litho-core crate, glue sources, and every
generated datapack (smoke, v0 and v1-dev), embedded as Rust source (К6, Д-112):
the client hook copies them into the new world's pack folder, so the Data Packs
screen shows them without dragging zip files. It validates the pins and the
paths that horizon-mixins will resolve. Native compilation still happens in CI.
"""
import argparse
import json
from pathlib import Path
import tempfile
import shutil
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PINS = json.loads((ROOT / "tools/horizon-revisions.json").read_text())


def archive(source, destination):
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as output:
        for path in sorted(source.rglob("*")):
            if path.is_file():
                output.write(path, path.relative_to(source).as_posix())


def validate_pack(pack: Path) -> None:
    mcmeta = pack / "pack.mcmeta"
    if not mcmeta.is_file():
        raise ValueError(f"Нет pack.mcmeta в {pack}")
    for path in sorted(pack.rglob("*.json")):
        json.loads(path.read_text())
    json.loads(mcmeta.read_text())

    df = pack / "data/minecraft/worldgen/density_function/overworld"
    noise = pack / "data/litospherity/worldgen/noise"
    if pack.name.startswith("litospherity_v1-dev_"):
        # К4а/часть 1: полная I2 — offset, pv и теперь factor/jaggedness
        # из ядра, чтобы игра не добавляла свой рельеф (Д-106).
        for name in ("ridges_folded", "offset", "factor", "jaggedness"):
            if not (df / f"{name}.json").is_file():
                raise ValueError(f"В {pack.name} нет I2-поля {name}")
        for name in ("pv", "offset", "factor", "jaggedness"):
            if not any(noise.rglob(f"{name}.json")):
                raise ValueError(f"В {pack.name} нет шума {name}")
    elif pack.name.startswith("litospherity_v0_"):
        for name in ("ridges_folded", "offset", "factor", "jaggedness"):
            if (df / f"{name}.json").exists() or any(noise.rglob(f"{name}.json")):
                raise ValueError(f"Замороженный {pack.name} не должен менять {name}")


def verify_embedded(hnmod: Path, packs) -> None:
    """К6 (Д-112): паки обязаны быть внутри архива, а не рядом с ним."""
    with zipfile.ZipFile(hnmod) as source:
        try:
            text = source.read("mixins/litospherity/glue/packs_data.rs").decode("utf-8")
        except KeyError:
            raise ValueError(
                "в .hnmod нет glue/packs_data.rs: паки не встроены"
            ) from None
    for pack in packs:
        if f'"{pack.name}"' not in text:
            raise ValueError(f"в .hnmod нет встроенного пака {pack.name}")
        if not (pack / "pack.mcmeta").is_file():
            raise ValueError(f"нет pack.mcmeta в {pack}")
    if text.count('"pack.mcmeta"') < len(packs):
        raise ValueError("packs_data.rs: не у всех паков есть pack.mcmeta")
    print(f"Паки внутри .hnmod: {len(packs)} (отдельных .zip нет)")


def package(bundle, out):
    client = ROOT / "mod/litospherity"
    native = json.loads((client / "horizon-native.json").read_text())
    if native["horizon"] != PINS["horizon"]:
        raise ValueError("horizon-native.json: не то закрепление Horizon")
    if native["mixins"] != PINS["horizon_mixins"]:
        raise ValueError("horizon-native.json: не то закрепление horizon-mixins")
    if native["game"] != PINS["game"]:
        raise ValueError("horizon-native.json: не та версия игры")

    core_manifest = (bundle / "horizon-mixin.toml").read_text()
    worldgen = tomllib.loads(core_manifest)
    if worldgen["compatibility"]["horizon_revision"] != PINS["horizon"]:
        raise ValueError("Бандл собран под другую ревизию Horizon")
    client_manifest = tomllib.loads((client / "mixins/litospherity/horizon-mixin.toml").read_text())
    if client_manifest["compatibility"]["horizon_revision"] != PINS["horizon"]:
        raise ValueError("Клиентские хуки собраны под другую ревизию Horizon")

    # The client manifest owns [mod]/[compatibility]; append only worldgen entries.
    additions = core_manifest[core_manifest.index("[[crate]]") :]
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        staging = Path(temporary)
        shutil.copytree(client, staging, dirs_exist_ok=True)
        target = staging / "mixins/litospherity"
        for name in ("glue", "crates"):
            shutil.copytree(bundle / name, target / name, dirs_exist_ok=True)
        manifest = target / "horizon-mixin.toml"
        manifest.write_text(manifest.read_text() + "\n" + additions)
        parsed = tomllib.loads(manifest.read_text())
        for entry in parsed.get("module", []):
            if not (target / entry["source"]).is_file():
                raise ValueError(f"[[module]] {entry['id']}: нет {entry['source']}")
        for entry in parsed.get("injection", []):
            if not (target / entry["helper"]).is_file():
                raise ValueError(f"[[injection]] {entry['id']}: нет {entry['helper']}")
        for entry in parsed.get("crate", []):
            if not (target / entry["path"] / "Cargo.toml").is_file():
                raise ValueError(f"[[crate]] {entry['id']}: нет {entry['path']}/Cargo.toml")
        for path in sorted(staging.rglob("*.json")):
            json.loads(path.read_text())
        archive(staging, out / "litospherity.hnmod")

    packs = sorted(path for path in (bundle / "packs").iterdir() if path.is_dir())
    for pack in packs:
        validate_pack(pack)
    verify_embedded(out / "litospherity.hnmod", packs)
    print(f"Packaged mod with {len(packs)} embedded datapacks: {out}")
    print("Horizon / mixins pins: " + PINS["horizon"] + " / " + PINS["horizon_mixins"])
    print("Native compilation and in-game world generation still require Horizon.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, default=ROOT / "out/bundle")
    parser.add_argument("--out", type=Path, default=ROOT / "out/release")
    args = parser.parse_args()
    package(args.bundle, args.out)
