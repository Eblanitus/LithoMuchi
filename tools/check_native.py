#!/usr/bin/env python3
"""Plan/apply the exact release bundle; report actionable CI annotations."""
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def run(command):
    env = dict(os.environ, CARGO_TERM_COLOR='never')
    result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, env=env)
    print(result.stdout, flush=True)
    if result.returncode:
        clean = re.sub(r'\x1b\[[0-9;]*m', '', result.stdout)
        tail = clean[-2500:].replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
        print(f'::error title=Native validation failed::{tail}', flush=True)
        raise SystemExit(result.returncode)


def main():
    archive = ROOT / 'out/release/litospherity.hnmod'
    destination = ROOT / 'out/native-check'
    if destination.exists():
        shutil.rmtree(destination)
    with zipfile.ZipFile(archive) as bundle:
        bundle.extractall(destination)
        metadata = json.loads(bundle.read('horizon-native.json'))
    horizon = ROOT / 'out/horizon-source'
    mixins = ROOT / 'out/horizon-mixins'
    # Fail if metadata and CI's checked-out pins drift apart.
    for source, pin in ((horizon, metadata['horizon']), (mixins, metadata['mixins'])):
        head = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
        if head != pin:
            raise SystemExit(f'Pin mismatch: {source}: {head} != {pin}')
    with tempfile.TemporaryDirectory(prefix='litho-packs-') as temporary:
        target = Path(temporary) / 'packs'
        shutil.copytree(destination / 'mixins/litospherity/crates/litho-packs', target)
        run(['cargo', 'test', '--manifest-path', str(target / 'Cargo.toml')])
    common = ['--workspace', str(horizon), '--mods', str(destination / 'mixins')]
    cli = ['cargo', 'run', '--manifest-path', str(mixins / 'Cargo.toml'),
           '-p', 'horizon-mixins-cli', '--']
    run(cli + ['plan'] + common + ['--out', str(ROOT / 'out/native-plan.json')])
    run(cli + ['apply'] + common + ['--out', str(ROOT / 'out/native-stage')])
    # The launcher reads the archive before invoking Mixins. Exercise that exact
    # reader too; plan/apply alone does not enforce its native-source allowlist.
    reader = ROOT / 'out/native-reader'
    (reader / 'src').mkdir(parents=True, exist_ok=True)
    dependency = ROOT / 'out/native-stage/crates/horizon-mods'
    (reader / 'Cargo.toml').write_text(
        '[package]\nname="litho-native-reader-check"\nversion="0.0.0"\nedition="2024"\n'
        '[workspace]\n[dependencies]\nhorizon-mods = { path = '
        + json.dumps(str(dependency)) + ', features = ["index"] }\n')
    (reader / 'src/main.rs').write_text(
        'fn main() {\n'
        ' let path = std::env::args().nth(1).expect("archive path");\n'
        ' horizon_mods::index::native::Bundle::read(std::path::Path::new(&path))\n'
        '   .unwrap_or_else(|e| panic!("Launcher rejected archive: {e}"));\n'
        ' assert!(!horizon_mods::index::native::allowed(\n'
        '   "crates/litho-packs/data/gameplay/active.json"));\n'
        ' println!("PASS: actual Horizon launcher Bundle::read");\n}\n')
    run(['cargo', 'run', '--manifest-path', str(reader / 'Cargo.toml'),
         '--target-dir', str(ROOT / 'out/native-stage/target'), '--', str(archive)])
    # Bundled path crates add workspace-package entries. Refresh those entries
    # while retaining pinned registry dependencies, then require a locked build.
    run(['cargo', 'update', '--workspace', '--manifest-path', str(ROOT / 'out/native-stage/Cargo.toml')])
    # Check the transformed libraries, not only the TOML and injection planner.
    run(['cargo', 'check', '--locked', '--manifest-path', str(ROOT / 'out/native-stage/Cargo.toml'),
         '--lib', '-p', 'horizon-core', '-p', 'horizon-world', '-p', 'horizon-client-render',
         '-p', 'horizon-client'])


if __name__ == '__main__':
    main()
