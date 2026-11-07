#!/usr/bin/env python3
"""Check the generated player archive, not only source templates."""
import json
from pathlib import Path
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
archive = ROOT / 'out/release/litospherity.hnmod'
with zipfile.ZipFile(archive) as z:
    names = z.namelist()
    assert len(names) == len(set(names)), 'duplicate archive entries'
    assert len(names) <= 512, 'registry file limit'
    assert sum(i.file_size for i in z.infolist()) <= 64 * 1024 * 1024, 'registry size limit'
    assert all('\\' not in n and '..' not in Path(n).parts and not n.startswith('/') for n in names)
    assert all(n == 'horizon-native.json' or n.startswith('mixins/litospherity/') for n in names)
    assert not any('smoke' in n.lower() for n in names), 'smoke pack must not be embedded'
    root = 'mixins/litospherity/'
    metadata = json.loads(z.read('horizon-native.json'))
    manifest = tomllib.loads(z.read(root + 'horizon-mixin.toml').decode())
    assert metadata['version'] == manifest['mod']['version'] == '0.2.0'
    assert metadata['horizon'] == manifest['compatibility']['horizon_revision']
    assert {c['dependent'] for c in manifest['crate']} == {'horizon-core', 'horizon-client-render'}
    for mode in ('gameplay', 'scientific'):
        data = root + f'crates/litho-packs/data/{mode}/'
        assert z.read(data + 'pack.png').startswith(b'\x89PNG\r\n\x1a\n')
        json.loads(z.read(data + 'pack.mcmeta'))
        for n in names:
            if n.startswith(data) and n.endswith('.json'):
                json.loads(z.read(n))
        lava = json.loads(z.read(data + f'data/litospherity/worldgen/density_function/v0/{mode}/lava.json'))
        assert (lava['xz_scale'], lava['y_scale']) == (64.0, 40.0)
    for injection in manifest['injection']:
        assert root + injection['helper'] in names
print('PASS: player archive paths, registry limits, pins, modes, icons, JSON and lava scales')
