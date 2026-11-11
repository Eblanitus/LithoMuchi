#!/usr/bin/env bash
# Локальная сборка и тесты БЕЗ установленного Rust: rustc 1.98.1, собранный в
# WebAssembly (npm-пакет @ai-ecoverse/wasi-rustc), запускается под Node WASI.
# Результат — программы wasm32-wasip1, они тоже идут под Node.
#
# Это инструмент разработки для песочниц без rustup. Приёмочный путь — CI
# (нативные Linux/Windows); здесь только быстрый цикл «правка → тест → метрика».
# Арифметика ядра детерминирована (свои таблицы, без libm), поэтому числа
# совпадают с нативными побитово; время в wasm примерно в 1.5–2 раза хуже.
#
#   tools/wasi-dev/lrust.sh setup                 # скачать компилятор (62 МБ)
#   tools/wasi-dev/lrust.sh build                 # ядро + песочница + xtask
#   tools/wasi-dev/lrust.sh test [core|sandbox|tests|xtask] [фильтр…]
#   tools/wasi-dev/lrust.sh sandbox <аргументы litho-sandbox>
#   tools/wasi-dev/lrust.sh xtask <аргументы xtask>
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
TC=${LITHO_WASI_RUST:-$HOME/.cache/wasi-rust}
OUT=${LITHO_WASI_OUT:-$ROOT/target/wasi}
PKG=@ai-ecoverse/wasi-rustc
VER=1.98.1-10
NODE=(node --no-warnings)

export WASI_DIRS="/tmp:$TC/package:$ROOT"
export WASIENV_RUSTC_SYSROOT="$TC/package"

rustc() {
  "${NODE[@]}" --stack-size=60000 "$HERE/run-threads.mjs" "$TC/package/bin/rustc.wasm" \
    --sysroot="$TC/package" --target=wasm32-wasip1 --edition 2024 "$@"
}
# --no-incremental-marking: V8 в Node 22 падает с SIGSEGV при обходе стека
# во время инкрементальной разметки на долгих wasm-прогонах (найдено ptrace).
run() { "${NODE[@]}" --no-incremental-marking "$HERE/run.mjs" "$@"; }

# Пересобирать только если исходники новее результата.
stale() {
  local out=$1; shift
  [ -e "$out" ] || return 0
  [ -n "$(find "$@" -newer "$out" -type f -print -quit 2>/dev/null)" ]
}

OPT=(-C opt-level=3 -C codegen-units=4 -C debuginfo=0)
# `cargo test` в CI идёт в debug: проверки переполнения и debug_assert включены.
TESTOPT=(-C opt-level=2 -C codegen-units=4 -C debuginfo=0 -C debug-assertions=on -C overflow-checks=on)
CORE="$ROOT/crates/litho-core"
SBX="$ROOT/crates/litho-sandbox"
LT="$ROOT/crates/litho-tests"
XT="$ROOT/tools/xtask"

build_core() {
  mkdir -p "$OUT"
  if stale "$OUT/liblitho_core.rlib" "$CORE/src"; then
    echo "· litho-core" >&2
    rustc "${OPT[@]}" --crate-type rlib --crate-name litho_core "$CORE/src/lib.rs" -o "$OUT/liblitho_core.rlib"
  fi
}
build_sandbox() {
  build_core
  if stale "$OUT/litho-sandbox.wasm" "$SBX/src" "$OUT/liblitho_core.rlib"; then
    echo "· litho-sandbox" >&2
    rustc "${OPT[@]}" --crate-type bin --crate-name litho_sandbox "$SBX/src/main.rs" \
      --extern "litho_core=$OUT/liblitho_core.rlib" -o "$OUT/litho-sandbox.wasm"
  fi
}
build_xtask() {
  build_core
  if stale "$OUT/xtask.wasm" "$XT/src" "$OUT/liblitho_core.rlib"; then
    echo "· xtask" >&2
    WASIENV_CARGO_MANIFEST_DIR="$XT" \
      rustc "${OPT[@]}" --crate-type bin --crate-name xtask "$XT/src/main.rs" \
      --extern "litho_core=$OUT/liblitho_core.rlib" -o "$OUT/xtask.wasm"
  fi
}

test_core() {
  if stale "$OUT/t-core.wasm" "$CORE/src"; then
    echo "· тесты litho-core" >&2
    rustc "${TESTOPT[@]}" --test --crate-name litho_core "$CORE/src/lib.rs" -o "$OUT/t-core.wasm"
  fi
  run "$OUT/t-core.wasm" "$@"
}
test_sandbox() {
  build_core
  if stale "$OUT/t-sandbox.wasm" "$SBX/src" "$OUT/liblitho_core.rlib"; then
    echo "· тесты litho-sandbox" >&2
    rustc "${TESTOPT[@]}" --test --crate-name litho_sandbox "$SBX/src/main.rs" \
      --extern "litho_core=$OUT/liblitho_core.rlib" -o "$OUT/t-sandbox.wasm"
  fi
  run "$OUT/t-sandbox.wasm" "$@"
}
test_tests() {
  # CI гоняет litho-tests в release (T115): здесь так же.
  build_core
  if stale "$OUT/t-tests.wasm" "$LT/src" "$ROOT/golden" "$OUT/liblitho_core.rlib"; then
    echo "· тесты litho-tests" >&2
    rustc "${OPT[@]}" --test --crate-name litho_tests "$LT/src/lib.rs" \
      --extern "litho_core=$OUT/liblitho_core.rlib" -o "$OUT/t-tests.wasm"
  fi
  if stale "$OUT/t-bridge.wasm" "$LT/tests" "$ROOT/mod/glue" "$OUT/liblitho_core.rlib"; then
    echo "· тест моста Horizon" >&2
    rustc "${OPT[@]}" --test --crate-name horizon_bridge "$LT/tests/horizon_bridge.rs" \
      --extern "litho_core=$OUT/liblitho_core.rlib" -o "$OUT/t-bridge.wasm"
  fi
  run "$OUT/t-tests.wasm" "$@"
  run "$OUT/t-bridge.wasm" "$@"
}
test_xtask() {
  build_core
  if stale "$OUT/t-xtask.wasm" "$XT/src" "$OUT/liblitho_core.rlib"; then
    echo "· тесты xtask" >&2
    WASIENV_CARGO_MANIFEST_DIR="$XT" rustc "${TESTOPT[@]}" --test --crate-name xtask "$XT/src/main.rs" \
      --extern "litho_core=$OUT/liblitho_core.rlib" -o "$OUT/t-xtask.wasm"
  fi
  run "$OUT/t-xtask.wasm" "$@"
}

cmd=${1:-help}; shift || true
case "$cmd" in
  setup)
    mkdir -p "$TC"
    if [ ! -e "$TC/package/bin/rustc.wasm" ]; then
      url="https://registry.npmjs.org/$PKG/-/wasi-rustc-$VER.tgz"
      echo "скачиваю $url" >&2
      curl -fsSL -o "$TC/rustc.tgz" "$url"
      tar -xzf "$TC/rustc.tgz" -C "$TC"
      rm -f "$TC/rustc.tgz"
    fi
    rustc --version
    ;;
  rustc) rustc "$@" ;;
  build) build_sandbox; build_xtask ;;
  test)
    what=${1:-all}; [ $# -gt 0 ] && shift
    case "$what" in
      core) test_core "$@" ;;
      sandbox) test_sandbox "$@" ;;
      tests) test_tests "$@" ;;
      xtask) test_xtask "$@" ;;
      all) test_core "$@"; test_sandbox "$@"; test_xtask "$@"; test_tests "$@" ;;
      *) echo "неизвестный набор тестов: $what" >&2; exit 2 ;;
    esac
    ;;
  sandbox) build_sandbox; run "$OUT/litho-sandbox.wasm" "$@" ;;
  xtask) build_xtask; run "$OUT/xtask.wasm" "$@" ;;
  *) sed -n '2,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' ;;
esac
