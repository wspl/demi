"""Run selected upstream checks: python3 verify.py SOURCE_DIR TJS_BINARY.

This is a runtime size experiment, not a Web Platform Tests conformance suite.
Each test has its own process group so timeouts also stop child processes.
"""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

source, binary = (Path(value).resolve() for value in sys.argv[1:3])
tests = [
    "fs-streams", "fs-stat", "fs-sync", "readfile",
    "text-coding", "text-encode-into", "abort-controller",
    "fetch-transform-stream", "fetch-stream-request", "fetch-stream-response",
    "fetch-stream-body-used", "fetch-stream-abort", "fetch-proxy-auth",
    "web-streams-echo", "web-streams-abort", "web-streams-cancel-close",
    "pipe", "pipe-abstract", "tcp-connect-abort", "tls-ca",
    "tls-connect-listen", "ws-headers", "dispose-subprocess", "proc-kill",
    "hashing", "webcrypto-hmac",
]

results = []
for name in tests:
    started = time.monotonic()
    process = subprocess.Popen(
        [str(binary), "run", str(source / "tests" / f"test-{name}.js")],
        cwd=source,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    try:
        output, _ = process.communicate(timeout=25)
        code = process.returncode
    except subprocess.TimeoutExpired:
        code = "timeout"
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        remaining, _ = process.communicate()
        if code == "timeout":
            output = remaining

    result = {
        "test": name,
        "exit": code,
        "seconds": round(time.monotonic() - started, 3),
    }
    if code != 0:
        result["output"] = output.decode(errors="replace")[-3000:]
    results.append(result)
    print(json.dumps(result), flush=True)

print(json.dumps({"passed": sum(r["exit"] == 0 for r in results), "total": len(results)}))
sys.exit(any(r["exit"] != 0 for r in results))
