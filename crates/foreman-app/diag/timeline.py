# m3/1: one line per transcript entry of a session Foreman launched: seconds since -since (ISO, UTC),
# type, and the tool calls / tool results / queued text it carries (truncated).
#   python crates/foreman-app/diag/timeline.py <transcript.jsonl> [--since 2026-10-23T12:22:03.467Z]
import json
import sys
from datetime import datetime


def ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00"))


path = sys.argv[1]
since = ts(sys.argv[sys.argv.index("--since") + 1]) if "--since" in sys.argv else None
for raw in open(path, encoding="utf-8"):
    try:
        e = json.loads(raw)
    except ValueError:
        continue
    t = e.get("timestamp")
    if not t:
        continue
    at = ts(t)
    if since and at < since:
        continue
    what = []
    msg = e.get("message") or {}
    content = msg.get("content")
    if isinstance(content, list):
        for c in content:
            if c.get("type") == "tool_use":
                what.append(f"tool_use {c.get('name')} {json.dumps(c.get('input'))[:140]}")
            elif c.get("type") == "tool_result":
                body = c.get("content")
                text = body if isinstance(body, str) else json.dumps(body)
                what.append(f"tool_result {text[:140]}")
            elif c.get("type") == "text":
                what.append(f"text {c.get('text', '')[:140]!r}")
    elif isinstance(content, str):
        what.append(f"text {content[:140]!r}")
    if e.get("type") == "queue-operation":
        what.append(f"{e.get('operation')} {str(e.get('content'))[:140]!r}")
    rel = f"{(at - since).total_seconds():7.2f}" if since else at.isoformat()
    print(rel, e.get("type"), " | ".join(what))
