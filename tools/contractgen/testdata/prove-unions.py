"""Prove union boundary tests fail for planted faults; restore each file."""

import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
CODEC = ROOT / "tools/contractgen/testdata/unions/contract_gen.go"
GENERATOR = ROOT / "tools/contractgen"


def replace(old, new):
    """Require a live mutation site before replacing it."""
    def change(source):
        if old not in source:
            raise RuntimeError(f"missing mutation site: {old}")
        return source.replace(old, new, 1)
    return change


CASES = [
    ("boolean tags", CODEC, replace("value != true", "value != false"), "TestBooleanReplies"),
    ("JSON declaration order", CODEC,
     replace("contract.Decode[ZFirst](data)", "contract.Decode[ASecond](data)"),
     "TestUntaggedDeclarationOrder"),
    ("MessagePack declaration order", CODEC,
     replace("contract.DecodeMsgpack[ZFirst](data)", "contract.DecodeMsgpack[ASecond](data)"),
     "TestUntaggedDeclarationOrder"),
    ("omit empty collections", CODEC,
     replace("if len(v.Resources) > 0 {", "if len(v.Resources) >= 0 {"),
     "TestAbsentCollections"),
    ("root reachability", GENERATOR / "unions.go",
     replace("g.order = order", "_ = order"), "TestUnionGeneration"),
    ("private schemas", GENERATOR / "typescript.go",
     replace('export = ""', 'export = "export "'), "TestUnionGeneration"),
    ("initialism names", GENERATOR / "names.go",
     replace('return strings.Join(parts, "")', 'return name'), "TestUnionGeneration"),
]

for label, path, mutate, test in CASES:
    original = path.read_text()
    try:
        path.write_text(mutate(original))
        result = subprocess.run(
            ["go", "test", "./tools/contractgen", "-run", f"^{test}$", "-count=1"],
            cwd=ROOT,
            env={**os.environ, "CGO_ENABLED": "0", "GOFLAGS": "-mod=readonly"},
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode == 0 or "--- FAIL:" not in result.stdout:
            raise RuntimeError(f"{label}: expected behavioral failure\n{result.stdout}{result.stderr}")
        print(f"detected: {label}", flush=True)
    finally:
        path.write_text(original)
