#!/usr/bin/env python3
"""Plan/apply the exact release bundle; report actionable CI annotations."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def run(command):
    result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT)
    print(result.stdout, flush=True)
    if result.returncode:
        tail = result.stdout[-12000:].replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
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
    # Check the transformed libraries, not only the TOML and injection planner.
    run(['cargo', 'check', '--locked', '--manifest-path', str(ROOT / 'out/native-stage/Cargo.toml'),
         '--lib', '-p', 'horizon-core', '-p', 'horizon-world', '-p', 'horizon-client-render',
         '-p', 'horizon-client'])


if __name__ == '__main__':
    main()
