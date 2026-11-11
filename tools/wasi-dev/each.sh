#!/bin/bash
# Запуск тестов по одному. В wasm паника = abort всего бинарника, поэтому
# обычный прогон останавливается на первом падении. Здесь каждый тест идёт
# отдельным процессом и падения собираются в список.
#   tools/wasi-dev/each.sh core|sandbox|tests|bridge|xtask [фильтр]
set -u
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/target/wasi"
suite="${1:-core}"; filter="${2:-}"
case "$suite" in
  core) bin=t-core.wasm ;; sandbox) bin=t-sandbox.wasm ;; tests) bin=t-tests.wasm ;;
  bridge) bin=t-bridge.wasm ;; xtask) bin=t-xtask.wasm ;; *) echo "неизвестный набор $suite" >&2; exit 2 ;;
esac
run() { WASI_DIRS="/tmp:$ROOT" node --no-warnings --no-incremental-marking --stack-size=60000 "$ROOT/tools/wasi-dev/run.mjs" "$OUT/$bin" "$@"; }
cd "$ROOT"
mapfile -t names < <(run --list 2>/dev/null | sed -n 's/: test$//p' | grep -- "$filter")
fail=0
for name in "${names[@]}"; do
  if ! log=$(run --exact "$name" --nocapture 2>&1); then
    fail=$((fail+1))
    echo "FAIL $name"
    echo "$log" | grep -v '^    at \|^$\|^wasm://\|^Node.js\|RUST_BACKTRACE\|^running\|RuntimeError' | sed 's/^/    /' | head -12
  fi
done
echo "итог $suite: тестов ${#names[@]}, падений $fail"
[ "$fail" -eq 0 ]
