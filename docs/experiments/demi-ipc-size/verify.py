"""Check byte-preserving IPC probes against a fragmenting Unix-socket peer.

Usage: python3 verify.py executable [executable ...]
No models or backend services are used.
"""
import os
import socket
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

payload = bytes(range(256)) * (3 * 1024 * 1024 // 256)


def roundtrip(binary, data, regular_files=False):
    with tempfile.TemporaryDirectory(prefix="ipc-size-", dir="/tmp") as work:
        endpoint = str(Path(work, "socket"))
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(endpoint)
        listener.listen(1)
        listener.settimeout(20)
        errors = []

        def serve():
            try:
                connection, _ = listener.accept()
                with connection:
                    connection.settimeout(20)
                    while chunk := connection.recv(7919):
                        connection.sendall(chunk)
            except Exception as error:
                errors.append(error)
            finally:
                listener.close()

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        args = [str(Path(binary).resolve()), endpoint]
        if regular_files:
            source = Path(work, "stdin")
            target = Path(work, "stdout")
            source.write_bytes(data)
            with source.open("rb") as stdin, target.open("wb") as stdout:
                result = subprocess.run(args, stdin=stdin, stdout=stdout,
                                        stderr=subprocess.PIPE, timeout=25)
            output = target.read_bytes()
        else:
            result = subprocess.run(args, input=data, capture_output=True, timeout=25)
            output = result.stdout
        thread.join(21)
        assert not thread.is_alive(), "server did not finish"
        assert not errors, errors
        assert result.returncode == 0, (result.returncode, result.stderr)
        assert output == data, (len(output), len(data), result.stderr)


for binary in sys.argv[1:]:
    for data, regular in [(b"", False), (b"hello\x00\xff\r\n", False),
                          (payload, False), (payload, True)]:
        roundtrip(binary, data, regular)
    missing = subprocess.run([str(Path(binary).resolve()), "/tmp/demi-ipc-size-no-such-socket"],
                             capture_output=True, timeout=5)
    assert missing.returncode != 0
    print(f"PASS {binary}: empty/binary/3 MiB pipe/3 MiB file/missing endpoint", flush=True)
