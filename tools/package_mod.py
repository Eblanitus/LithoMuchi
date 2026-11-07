#!/usr/bin/env python3
"""Package xtask's worldgen bundle together with the client hooks (stdlib only)."""
import argparse
import json
from pathlib import Path
import tempfile
import shutil
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def archive(source, destination):
    with zipfile.ZipFile(destination, 'w', zipfile.ZIP_DEFLATED) as output:
        for path in sorted(source.rglob('*')):
            if path.is_file():
                output.write(path, path.relative_to(source).as_posix())


def package(bundle, out, include_smoke=False):
    client = ROOT / 'mod/litospherity'
    core_manifest = (bundle / 'horizon-mixin.toml').read_text()
    metadata = json.loads((client / 'horizon-native.json').read_text())
    worldgen = tomllib.loads(core_manifest)
    if worldgen['compatibility']['horizon_revision'] != metadata['horizon']:
        raise ValueError('Client and worldgen Horizon revisions differ')
    # The client manifest owns [mod]/[compatibility]; append only worldgen entries.
    additions = core_manifest[core_manifest.index('[[crate]]'):]
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        staging = Path(temporary)
        shutil.copytree(client, staging, dirs_exist_ok=True)
        target = staging / 'mixins/litospherity'
        for license_name in ('LICENSE-MIT', 'LICENSE-APACHE'):
            shutil.copyfile(ROOT / license_name, target / license_name)
        for name in ('glue', 'crates'):
            shutil.copytree(bundle / name, target / name)
        manifest = target / 'horizon-mixin.toml'
        manifest.write_text(manifest.read_text() + '\n' + additions)
        parsed = tomllib.loads(manifest.read_text())
        for entry in parsed.get('module', []):
            assert (target / entry['source']).is_file(), entry
        for entry in parsed.get('injection', []):
            assert (target / entry['helper']).is_file(), entry
        for entry in parsed.get('crate', []):
            assert (target / entry['path'] / 'Cargo.toml').is_file(), entry
        archive(staging, out / 'litospherity.hnmod')
    # A player release is one self-contained mod, not separate world-mode ZIPs.
    names = ['litospherity_smoke'] if include_smoke else []
    for name in ('litospherity_v0_gameplay', 'litospherity_v0_scientific', 'litospherity_smoke'):
        if name not in names:
            # Also clean outputs from older packager versions on rebuild.
            (out / (name + '.zip')).unlink(missing_ok=True)
    for name in names:
        pack = bundle / 'packs' / name
        for path in pack.rglob('*.json'):
            json.loads(path.read_text())
        json.loads((pack / 'pack.mcmeta').read_text())
        archive(pack, out / (name + '.zip'))
    print(f'Packaged self-contained mod with two embedded modes: {out / "litospherity.hnmod"}')
    if include_smoke:
        print(f'Developer-only smoke ZIP: {out / "litospherity_smoke.zip"}')
    print('Native compilation and in-game world generation still require Horizon.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, default=ROOT / 'out/bundle')
    parser.add_argument('--out', type=Path, default=ROOT / 'out/release')
    parser.add_argument('--include-smoke', action='store_true', help='Developer smoke ZIP (never embedded)')
    args = parser.parse_args()
    package(args.bundle, args.out, args.include_smoke)
