#!/usr/bin/env python3
"""Turn cargo JSON/text diagnostics into GitHub annotations.

The sandbox can read check annotations through api.github.com, while raw CI
logs live on a different host. This keeps compiler/test failures actionable
without weakening -D warnings or swallowing cargo's exit code (use pipefail).
"""
import json
import re
import sys


def escape(text):
    return str(text).replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def emit(path, line, message, level="error"):
    print(f"::{level} file={path},line={line}::{escape(message)}", flush=True)


mode = sys.argv[1] if len(sys.argv) > 1 else "clippy"
pending = None
report = []
for raw in sys.stdin:
    sys.stdout.write(raw)
    sys.stdout.flush()
    if mode == "report":
        report.append(raw.rstrip())
        continue
    if mode == "clippy":
        try:
            item = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if item.get("reason") != "compiler-message":
            continue
        message = item.get("message", {})
        if message.get("level") not in {"error", "warning"}:
            continue
        spans = message.get("spans") or []
        span = next((s for s in spans if s.get("is_primary")), spans[0] if spans else {})
        text = message.get("message", "")
        rendered = message.get("rendered")
        if rendered:
            text = rendered
        emit(span.get("file_name", ".github"), span.get("line_start", 1), text, message["level"])
    else:
        match = re.search(r"panicked at ([^:]+):(\d+):\d+:", raw)
        if match:
            pending = (match.group(1), int(match.group(2)), [])
            continue
        if pending is not None:
            if raw.startswith("test ") or raw.startswith("failures:") or raw.startswith("note:"):
                emit(pending[0], pending[1], "\n".join(pending[2]) or "test panic")
                pending = None
            else:
                pending[2].append(raw.rstrip())
        match = re.search(r"^error(?:\[[^\]]+\])?: (.*)", raw)
        if match:
            emit(".github", 1, match.group(0))
if pending is not None:
    emit(pending[0], pending[1], "\n".join(pending[2]) or "test panic")
if mode == "report" and report:
    emit(".github", 1, "\n".join(report), "notice")
