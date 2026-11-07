#!/usr/bin/env python3
"""Check the generated player archive, not only source templates."""
import json
import re
from pathlib import Path
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
archive = ROOT / 'out/release/litospherity.hnmod'
for mode in ('gameplay', 'scientific'):
    assert not (archive.parent / f'litospherity_v0_{mode}.zip').exists(), 'redundant mode ZIP in player release'
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
    # Match the launcher's native source allowlist, including its depth limits.
    for name in names:
        if name == 'horizon-native.json':
            continue
        rest = name.removeprefix(root)
        parts = rest.split('/')
        data = len(parts) >= 2 and (parts[0] == 'data' or (len(parts) >= 3 and parts[1] == 'data'))
        text = len(parts) <= 2 and (parts[-1].startswith(('README', 'LICENSE', 'LICENCE', 'COPYING', 'NOTICE')) or parts[-1] == '.gitignore')
        assert rest.endswith(('.rs', '.toml')) or data or text, f'unexpected native source: {name}'
    source = z.read(root + 'crates/litho-packs/src/lib.rs').decode()
    assert 'include_bytes!' not in source
    entries = re.findall(r'^    \(("[^"\n]+"), ("[^"\n]+"), &(\[[0-9, ]*\])\),$', source, re.M)
    packs = {}
    for title, path, values in entries:
        key = (json.loads(title), json.loads(path))
        assert key not in packs, f'duplicate embedded pack entry: {key}'
        packs[key] = bytes(json.loads(values))
    for mode, title in (('gameplay', 'Litospherity - Игровой мир'), ('scientific', 'Litospherity - Научный мир')):
        original = ROOT / f'out/bundle/packs/litospherity_v0_{mode}'
        expected = {p.relative_to(original).as_posix(): p.read_bytes()
                    for p in original.rglob('*') if p.is_file()}
        embedded = {path: content for (pack, path), content in packs.items() if pack == title}
        assert expected and embedded == expected, f'embedded pack bytes differ: {mode}'
        assert packs[title, 'pack.png'].startswith(b'\x89PNG\r\n\x1a\n')
        json.loads(packs[title, 'pack.mcmeta'])
        for (pack, path), content in packs.items():
            if pack == title and path.endswith('.json'):
                json.loads(content)
        lava = json.loads(packs[title, f'data/litospherity/worldgen/density_function/v0/{mode}/lava.json'])
        assert (lava['xz_scale'], lava['y_scale']) == (64.0, 40.0)
    assert {title for title, path in packs} == {'Litospherity - Игровой мир', 'Litospherity - Научный мир'}
    for injection in manifest['injection']:
        assert root + injection['helper'] in names
print('PASS: player archive paths, registry limits, pins, modes, icons, JSON and lava scales')
