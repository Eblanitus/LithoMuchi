#!/usr/bin/env python3
"""Ф3 (docs/PLAN-FABRIC.md): ресурсы мода Fabric из пака Horizon v1-dev и ванильных JSON 26.3.

    python3 tools/fabric_resources.py <пак litospherity_v1-dev_gameplay> <server.jar 26.3> <out>

- density_function/overworld/* пака Horizon → те же файлы, узел шума litospherity:* заменён на
  {"type": "litospherity:field", "field": ...}; обёртка minecraft:cache та же (формат 26.3 = Horizon).
- noise_settings litospherity:gameplay / scientific = ванильный overworld; aquifers.lava → litospherity:lava_aquifer;
  научный: высота 512 от y −64 (Д-113), свой dimension_type.
- world_preset litospherity:gameplay / scientific (кнопка «Тип мира») + тег normal.
Файлы Mojang в репозиторий не кладутся: берутся из server.jar при сборке.
"""
import io, json, sys, zipfile
from pathlib import Path

def vanilla(jar_path):
    z = zipfile.ZipFile(jar_path)
    inner = [n for n in z.namelist() if n.startswith("META-INF/versions/") and n.endswith(".jar")]
    if inner:
        z = zipfile.ZipFile(io.BytesIO(z.read(inner[0])))
    return lambda p: json.loads(z.read("data/minecraft/" + p))

def convert(node):
    if isinstance(node, dict):
        if node.get("type") == "minecraft:noise" and str(node.get("noise", "")).startswith("litospherity:"):
            return {"type": "litospherity:field", "field": node["noise"].rsplit("/", 1)[1]}
        return {k: convert(v) for k, v in node.items()}
    if isinstance(node, list):
        return [convert(v) for v in node]
    return node

def write(out, rel, data):
    p = out / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

def main(pack, jar, out):
    pack, out = Path(pack), Path(out)
    get = vanilla(jar)
    df = pack / "data/minecraft/worldgen/density_function"
    for f in sorted(df.rglob("*.json")):
        write(out, f"data/minecraft/worldgen/density_function/{f.relative_to(df).as_posix()}",
              convert(json.loads(f.read_text(encoding="utf-8"))))
    settings = get("worldgen/noise_settings/overworld.json")
    dim = get("dimension_type/overworld.json")
    preset = get("worldgen/world_preset/normal.json")
    for mode in ("gameplay", "scientific"):
        s = json.loads(json.dumps(settings))
        # Д-097: лава аквифера по ячейке, как хук Horizon на minecraft:aquifer_lava.
        s["aquifers"]["lava"] = {"type": "litospherity:lava_aquifer"}
        if mode == "scientific":
            s["noise"]["min_y"], s["noise"]["height"] = -64, 512
        write(out, f"data/litospherity/worldgen/noise_settings/{mode}.json", s)
        p = json.loads(json.dumps(preset))
        ow = p["dimensions"]["minecraft:overworld"]
        ow["generator"]["settings"] = f"litospherity:{mode}"
        if mode == "scientific":
            ow["type"] = "litospherity:scientific"
        write(out, f"data/litospherity/worldgen/world_preset/{mode}.json", p)
    d = dict(dim); d["min_y"], d["height"] = -64, 512
    write(out, "data/litospherity/dimension_type/scientific.json", d)
    write(out, "data/minecraft/tags/worldgen/world_preset/normal.json",
          {"replace": False, "values": ["litospherity:gameplay", "litospherity:scientific"]})
    for lang, g, sc in (("en_us", "Litospherity: gameplay", "Litospherity: scientific"),
                        ("ru_ru", "Литосферити: геймплейный", "Литосферити: научный")):
        write(out, f"assets/litospherity/lang/{lang}.json",
              {"generator.litospherity.gameplay": g, "generator.litospherity.scientific": sc})

if __name__ == "__main__":
    main(*sys.argv[1:4])
