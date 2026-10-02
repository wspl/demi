"""Prove each remaining marker at its generated Go or TypeScript boundary."""

import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
ENV = dict(os.environ, CGO_ENABLED="0", GOFLAGS="-mod=readonly")
CASES = [
    ("http-url scheme", "internal/contract/text.go", 'parsed.Scheme() != "https")', 'parsed.Scheme() != "https" && parsed.Scheme() != "ftp")', "TestFormatHTTPURL"),
    ("http-url canonicalization", "internal/contract/text.go", 'return parsed.Href(false), nil', 'return value, nil', "TestFormatHTTPURL"),
    ("http-url Zod", "tools/contractgen/typescript.go", 'z.url({ protocol: z.regexes.httpProtocol })', 'z.url()', "TestTableAndTextTypeScript"),
    ("email check", "internal/contract/text.go", 'if strings.HasPrefix(value, ".") || strings.Contains(value, "..") || !emailPattern.MatchString(value) {', 'if false && (strings.HasPrefix(value, ".") || strings.Contains(value, "..") || !emailPattern.MatchString(value)) {', "TestFormatEmail"),
    ("email lowercase", "internal/contract/text.go", 'value = strings.ToLower(Trim(value))', 'value = strings.ToUpper(Trim(value))', "TestFormatEmail"),
    ("trimmed decoding", "tools/contractgen/testdata/text/contract_gen.go", 'value = contract.Trim(value)', 'value = value', "TestFormatTrimmed"),
    ("email Zod", "tools/contractgen/typescript.go", 'code = "z.email().max(254)"', 'code = "z.string().max(254)"', "TestTableAndTextTypeScript"),
    ("trimmed Zod", "tools/contractgen/typescript.go", 'code += ".trim()"', 'code += ""', "TestTableAndTextTypeScript"),
    ("tuple scalar", "internal/contract/msgpack.go", 'if len(fields) == 1 {', 'if false && len(fields) == 1 {', "TestKeptTuple"),
    ("table lookups", "tools/contractgen/tables.go", 'out.WriteString(previewLookups)', 'out.WriteString("")', "TestTableAndTextTypeScript"),
    ("table values", "tools/contractgen/tables.go", 'constant.StringVal(v), nil', 'constant.StringVal(v) + "x", nil', "TestTableAndTextTypeScript"),
]

for name, filename, before, after, test in CASES:
    if len(sys.argv) > 1 and name not in sys.argv[1:]:
        continue
    path = ROOT / filename
    original = path.read_text()
    if before not in original:
        raise RuntimeError(f"mutation site absent: {name}")
    try:
        path.write_text(original.replace(before, after))
        result = subprocess.run(
            ["go", "test", "./tools/contractgen", "-count=1", "-run", f"^{test}$"],
            cwd=ROOT, env=ENV, capture_output=True, text=True, check=False,
        )
        output = result.stdout + result.stderr
        if result.returncode == 0 or "--- FAIL:" not in output:
            raise RuntimeError(f"{name}: no behavioral failure\n{output}")
        print(f"caught: {name}", flush=True)
    finally:
        path.write_text(original)
