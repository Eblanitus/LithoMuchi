#!/usr/bin/env bash
# Test against genuine Horizon source, without running mixins or the game.
# Pins come from tools/horizon-revisions.json (single source of truth).
set -euo pipefail
if [ "$#" -ne 2 ]; then
    echo 'Usage: run.sh HORIZON_SOURCE LIBM_SOURCE (libm crate 0.2.16)' >&2
    exit 2
fi
root=$(cd "$(dirname "$0")/../.." && pwd)
horizon=$(cd "$1" && pwd)
libm=$(cd "$2" && pwd)
expected=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["horizon"])' "$root/tools/horizon-revisions.json")
if [ -f "$horizon/.horizon-revision" ]; then
    revision=$(tr -d '\r\n' < "$horizon/.horizon-revision")
elif git -C "$horizon" rev-parse --verify HEAD >/dev/null 2>&1; then
    revision=$(git -C "$horizon" rev-parse HEAD)
else
    echo 'Horizon source must be a git checkout or have .horizon-revision' >&2
    exit 2
fi
if [ "$revision" != "$expected" ]; then
    echo "Wrong Horizon revision: $revision (need $expected)" >&2
    exit 2
fi
python3 - "$libm/Cargo.toml" "$root/tools/horizon-revisions.json" <<'PY'
import json
import sys
import tomllib

with open(sys.argv[1], 'rb') as f:
    package = tomllib.load(f)['package']
with open(sys.argv[2]) as f:
    pins = json.load(f)
if (package['name'], package['version']) != ('libm', pins['libm_version']):
    raise SystemExit(f"Need libm crate {pins['libm_version']}")
PY
stage=$(mktemp -d "${TMPDIR:-/tmp}/litho-horizon-test.XXXXXXXX")
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/crates"
cp -R "$horizon/crates/horizon-core" "$stage/crates/horizon-core"
cp -R "$root/crates/litho-core" "$stage/crates/litho-core"
cp -R "$libm" "$stage/libm"
cat > "$stage/Cargo.toml" <<'TOML'
[workspace]
members = ["crates/horizon-core", "crates/litho-core"]
exclude = ["libm"]
resolver = "2"
[workspace.package]
edition = "2024"
license = "MIT"
rust-version = "1.85"
[workspace.lints.clippy]
too_many_arguments = "allow"
type_complexity = "allow"
needless_range_loop = "allow"
large_enum_variant = "allow"
[patch.crates-io]
libm = { path = "libm" }
TOML
python3 - "$root" "$stage" <<'PY'
import pathlib
import sys

root, stage = map(pathlib.Path, sys.argv[1:])
core = stage / 'crates/horizon-core'
p = core / 'Cargo.toml'
s = p.read_text()
marker = '[dependencies]'
if s.count(marker) != 1:
    raise SystemExit('unexpected core dependencies')
p.write_text(s.replace(marker, marker + '\nlitho-core = { path = "../litho-core" }', 1))
p = core / 'src/lib.rs'
p.write_text(p.read_text() + '\npub mod litospherity_glue;\n')
(core / 'src/litospherity_glue.rs').write_text(
    (root / 'mod/glue/litospherity_glue.rs.in').read_text()
)
p = core / 'src/density/graph/seeded.rs'
s = p.read_text()
marker = '        let entry = graph.noise_entry(id)?;'
if s.count(marker) != 1:
    raise SystemExit('unexpected injection target')
helper = (root / 'mod/glue/noise_hook.rs.in').read_text()
s = s.replace(
    marker,
    helper
    + '\n        if let std::ops::ControlFlow::Break(result) = litospherity_noise(self, graph, id) { return result; }\n'
    + marker,
    1,
)
p.write_text(s)
tests = core / 'tests'
tests.mkdir(exist_ok=True)
(tests / 'litho_real.rs').write_text(
    (root / 'tools/horizon-core-check/real_density.rs').read_text()
)
PY
# This intentionally emulates the generated HEAD injection; it does NOT test
# horizon-mixins, the launcher, the world generator, datapacks, or Minecraft.
cargo test --offline --manifest-path "$stage/Cargo.toml" -p horizon-core --test litho_real
