"""Plant codec defects, require a behavioral test failure, and restore every file."""

import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
JSON = ROOT / "internal/contract/json.go"
MSGPACK = ROOT / "internal/contract/msgpack.go"
MAIN = ROOT / "tools/contractgen/main.go"
CHECK = ROOT / "tools/contractgen/check.go"
RULES = ROOT / "tools/contractgen/rules.go"
TS = ROOT / "tools/contractgen/typescript.go"
RUNNER = ROOT / "tools/contractgen/testdata/runner/contract_gen.go"
BLOCKS = ROOT / "tools/contractgen/testdata/blocks/contract_gen.go"


def replace(old, new):
    """Build one exact source mutation, refusing a stale mutation site."""
    def change(source):
        if old not in source:
            raise RuntimeError(f"mutation site absent: {old}")
        return source.replace(old, new)
    return change


def bypass_error(message):
    return replace(f'fmt.Errorf("{message}")', "nil")


cases = [
    ("JSON duplicate keys", JSON, replace("if seen[key] {", "if false && seen[key] {"), "TestGeneratedFeatures/.*duplicate"),
    ("UTF-8", JSON, replace("if !utf8.Valid(data) {", "if false && !utf8.Valid(data) {"), "TestGeneratedFeatures/utf8"),
    ("escaped Unicode", JSON, replace("if err := checkSurrogates(data); err != nil {", "if err := checkSurrogates(data); false && err != nil {"), "TestGeneratedFeatures/surrogate"),
    ("JSON null", JSON, replace("if IsNull(data) {", "if false && IsNull(data) {"), "TestBoundaryEdges/required_string_null"),
    ("JSON trailing", JSON, lambda s: s.replace('return errors.New("trailing JSON data")', "return nil").replace("err := json.Unmarshal(data, &value)", "err := json.NewDecoder(bytes.NewReader(data)).Decode(&value)"), "TestGeneratedFeatures/trailing"),
    ("field presence", BLOCKS, replace("if !ok {", "if false && !ok {"), "TestBlockCorpus/a_nullable_field_that_is_absent"),
    ("strict fields", BLOCKS, bypass_error("unknown field"), "TestBlockCorpus/an_unknown_field"),
    ("unknown tag", BLOCKS, replace('return nil, fmt.Errorf("unknown Block tag %q", tag)', "return nil, nil"), "TestBlockCorpus/a_block_kind"),
    ("Unicode length", JSON, replace("if n < minimum || maximum >= 0 && n > maximum {", "if false && (n < minimum || maximum >= 0 && n > maximum) {"), "TestBoundaryEdges/65_astral"),
    ("pattern", JSON, replace("if !matched {", "if false && !matched {"), "TestBlockCorpus/a_blob_reference_that_is_not_a_SHA"),
    ("numeric bounds", BLOCKS, bypass_error("outside numeric bounds"), "TestBlockCorpus/a_size_beyond"),
    ("enum", BLOCKS, bypass_error("unknown value"), "TestBlockCorpus/a_tool_call_status"),
    ("custom rule", BLOCKS, replace("if err := validateAgentMessageBlock(v); err != nil {", "if err := validateAgentMessageBlock(v); false && err != nil {"), "TestBlockCorpus/a_receipt"),
    ("timestamp precision", JSON, replace("if parsed.Format(layout) != value {", "if false && parsed.Format(layout) != value {"), "TestBoundaryEdges/comma_fraction"),
    ("base64 padding bits", JSON, lambda s: s.replace("base64.StdEncoding.Strict()", "base64.StdEncoding").replace("if base64.StdEncoding.EncodeToString(decoded) != value {", "if false && base64.StdEncoding.EncodeToString(decoded) != value {"), "TestGeneratedFeatures/base64"),
    ("MessagePack narrowing", MSGPACK, replace("if dst.OverflowUint(u) {", "if false && dst.OverflowUint(u) {"), "TestRunnerRefusals/overflow"),
    ("MessagePack binary", MSGPACK, replace("if target.Elem().Kind() != reflect.Uint8 || code != msgpcode.Bin8 && code != msgpcode.Bin16 && code != msgpcode.Bin32 {", "if false && (target.Elem().Kind() != reflect.Uint8 || code != msgpcode.Bin8 && code != msgpcode.Bin16 && code != msgpcode.Bin32) {"), "TestRunnerRefusals/string_bytes"),
    ("MessagePack duplicate", MSGPACK, lambda s: s.replace("if seen[key] {", "if false && seen[key] {").replace("if _, ok := out[key]; ok {", "if _, ok := out[key]; false && ok {"), "TestRunnerRefusals/duplicate_tag"),
    ("MessagePack trailing", MSGPACK, replace("if r.Len() != 0 {", "if false && r.Len() != 0 {"), "TestRunnerRefusals/trailing"),
    ("compact integers", MSGPACK, replace("UseCompactInts(true)", "UseCompactInts(false)"), "TestRunnerCorpus"),
    ("timestamp extension", MSGPACK, replace("if id != -1 ||", "if false && id != -1 ||"), "TestTimestampExtensions/wrong_extension"),
    ("timestamp milliseconds", MSGPACK, replace("nanos%1e6 != 0", "false"), "TestTimestampExtensions/fractional_milliseconds"),
    ("integer kind", MSGPACK, replace('return value, errors.New("expected integer")', 'return value, nil'), "TestRunnerRefusals/float_integer"),
    ("negative unsigned", MSGPACK, replace('return value, errors.New("negative unsigned integer")', 'return value, nil'), "TestRunnerRefusals/negative"),
    ("MessagePack field presence", RUNNER, replace('if !present {', 'if false && !present {'), "TestRunnerRefusals/missing_nullable"),
    ("MessagePack null", MSGPACK, replace('return value, errors.New("null or empty MessagePack")', 'return value, nil'), "TestRunnerRefusals/optional_null_string"),
    ("record order", MSGPACK, replace('return strings.Compare(a.String(), b.String())', 'return -strings.Compare(a.String(), b.String())'), "TestRecordEncoding"),
    ("unknown marker", MAIN, replace('out["!error"] = "unsupported marker: " + key', 'out["!error"] = ""'), "TestGenerationRefusals/unknown$"),
    ("unsupported shape", CHECK, replace('return fmt.Errorf("unsupported shape %s", t)', 'return nil'), "TestGenerationRefusals/unknown_shape"),
    ("pattern subset", RULES, replace('if refused {', 'if false && refused {'), "TestGenerationRefusals/bad_pattern"),
    ("safe web integer", TS, replace('if err != nil || maximum > 9007199254740991 {', 'if false && (err != nil || maximum > 9007199254740991) {'), "TestGenerationRefusals/unsafe_integer"),
    ("send strictness", TS, replace('g.err = fmt.Errorf("%s: send-only object must refuse unknown fields", name)', '_ = fmt.Errorf("%s: send-only object must refuse unknown fields", name)'), "TestGenerationRefusals/open_send"),

]

env = dict(os.environ, CGO_ENABLED="0", GOFLAGS="-mod=readonly", GOEXPERIMENT="none")
for name, path, mutation, test in cases:
    if len(sys.argv) > 1 and name not in sys.argv[1:]:
        continue
    generated_before = set((ROOT / "tools/contractgen/testdata/invalid").glob("*/contract_gen.go"))
    original = path.read_text()
    try:
        mutated = mutation(original)
        if mutated == original:
            raise RuntimeError(f"unchanged mutation: {name}")
        path.write_text(mutated)
        result = subprocess.run(
            ["go", "test", "./tools/contractgen", "-count=1", "-run", test],
            cwd=ROOT, env=env, capture_output=True, text=True, check=False,
        )
        output = result.stdout + result.stderr
        if result.returncode == 0 or "--- FAIL:" not in output:
            raise RuntimeError(f"{name}: no behavioral failure\n{output}")
        print(f"caught: {name}", flush=True)
    finally:
        path.write_text(original)
        for extra in set((ROOT / "tools/contractgen/testdata/invalid").glob("*/contract_gen.go")) - generated_before:
            extra.unlink()
