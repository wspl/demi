package core_test

import (
	"bytes"
	"encoding/json/v2"
	"errors"
	"os"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/core"
)

// These contract tests run entirely in memory, with a budget of one second
// per test. Fixtures are the Rust core's stored-data and browser corpus.

func fixture(t *testing.T, name string) []byte {
	t.Helper()
	data, err := os.ReadFile("testdata/" + name + ".json")
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func blocks(t *testing.T) []map[string]any {
	t.Helper()
	var values []map[string]any
	if err := json.Unmarshal(fixture(t, "blocks"), &values); err != nil {
		t.Fatal(err)
	}
	return values
}

func document(t *testing.T, value any) []byte {
	t.Helper()
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func sameJSON(t *testing.T, got, want []byte) {
	t.Helper()
	var a, b any
	if err := json.Unmarshal(got, &a); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(want, &b); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(a, b) {
		t.Fatalf("JSON mismatch\ngot: %s\nwant: %s", got, want)
	}
}

func TestStoredBlocks(t *testing.T) {
	for _, value := range blocks(t) {
		t.Run(value["id"].(string), func(t *testing.T) {
			data := document(t, value)
			block, err := core.Decode[core.Block](data)
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := core.Encode(block)
			if err != nil {
				t.Fatal(err)
			}
			sameJSON(t, encoded, data)
			if core.IsEditable(block) != (value["type"] == "user") {
				t.Fatal("editable type")
			}
			if core.BlockIDOf(block).String() != value["id"] || core.BlockCreatedAt(block).String() != value["createdAt"] {
				t.Fatal("block metadata")
			}
		})
	}
}

// mutate follows the shared Rust/browser fixture's JSON-pointer changes.
func mutate(t *testing.T, value any, pointer string, replacement any, remove bool) {
	t.Helper()
	parts := strings.Split(strings.TrimPrefix(pointer, "/"), "/")
	for _, part := range parts[:len(parts)-1] {
		switch v := value.(type) {
		case map[string]any:
			value = v[part]
		case []any:
			i, err := strconv.Atoi(part)
			if err != nil {
				t.Fatal(err)
			}
			value = v[i]
		default:
			t.Fatalf("bad fixture path %s", pointer)
		}
	}
	key := parts[len(parts)-1]
	switch v := value.(type) {
	case map[string]any:
		if remove {
			delete(v, key)
		} else {
			v[key] = replacement
		}
	case []any:
		i, err := strconv.Atoi(key)
		if err != nil {
			t.Fatal(err)
		}
		v[i] = replacement
	default:
		t.Fatalf("bad fixture parent %s", pointer)
	}
}

func TestStoredBlockRefusals(t *testing.T) {
	var table struct {
		Refused []struct {
			Why     string         `json:"why"`
			Fixture string         `json:"fixture"`
			Set     map[string]any `json:"set"`
			Remove  []string       `json:"remove"`
		} `json:"refused"`
	}
	if err := json.Unmarshal(fixture(t, "blocks-mutations"), &table); err != nil {
		t.Fatal(err)
	}
	for _, test := range table.Refused {
		t.Run(test.Why, func(t *testing.T) {
			for _, value := range blocks(t) {
				if value["id"] != test.Fixture {
					continue
				}
				for _, path := range test.Remove {
					mutate(t, value, path, nil, true)
				}
				for path, replacement := range test.Set {
					mutate(t, value, path, replacement, false)
				}
				if _, err := core.Decode[core.Block](document(t, value)); err == nil {
					t.Fatal("accepted invalid block")
				}
				return
			}
			t.Fatal("missing fixture")
		})
	}
}

func TestEncodingPrimitives(t *testing.T) {
	binary := core.NewB64Bytes([]byte{0, 1, 2, 255})
	encoded, err := core.Encode(binary)
	if err != nil || string(encoded) != `"AAEC/w=="` {
		t.Fatalf("base64: %s %v", encoded, err)
	}
	decoded, err := core.Decode[core.B64Bytes](encoded)
	if err != nil || !bytes.Equal(decoded.Bytes(), binary.Bytes()) || binary.Base64Len() != 8 {
		t.Fatal("base64 round trip", err)
	}
	for _, bad := range []string{`"AAEC/w"`, `"AAEC_w=="`, `"***"`, `null`, `[0]`, `"AB=="`, `"AA\n=="`} {
		if _, err := core.Decode[core.B64Bytes]([]byte(bad)); err == nil {
			t.Fatal("accepted base64", bad)
		}
	}
	instant, err := core.TimestampFromMillisecond(1790000000000)
	if err != nil {
		t.Fatal(err)
	}
	for _, text := range []string{"2026-09-21T14:13:20Z", "2026-09-21T14:13:20.000Z", "2026-09-21T16:13:20+02:00"} {
		got, err := core.Decode[core.Timestamp](document(t, text))
		if err != nil || got != instant {
			t.Fatalf("time %s: %v", text, err)
		}
	}
	encoded, err = core.Encode(instant)
	if err != nil || string(encoded) != `"2026-09-21T14:13:20.000Z"` {
		t.Fatal("time encoding", string(encoded), err)
	}
	for _, bad := range []string{`"2026-09-21T14:13:20.0001Z"`, `"2026-09-21"`, `"yesterday"`, `1790000000000`} {
		if _, err := core.Decode[core.Timestamp]([]byte(bad)); err == nil {
			t.Fatal("accepted time", bad)
		}
	}
	if core.TruncateTimestamp(time.Unix(1790000000, 999999)) != instant {
		t.Fatal("time truncation")
	}
	for _, bad := range []string{`""`, `7`, `null`} {
		if _, err := core.Decode[core.BlockID]([]byte(bad)); err == nil {
			t.Fatal("accepted identity", bad)
		}
	}
	id, err := core.ParseBlockID(" b\x00 ")
	if err != nil || id.String() != " b\x00 " {
		t.Fatal("identity changed", err)
	}
	if _, err := core.Encode(core.BlockID{}); err == nil {
		t.Fatal("encoded zero identity")
	}
	digest := "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
	if core.BlobRefOf([]byte("test")).String() != digest {
		t.Fatal("blob digest")
	}
	for _, bad := range []string{strings.ToUpper(digest), digest[1:], digest + "0", "sha-1"} {
		if _, err := core.Decode[core.BlobRef](document(t, bad)); err == nil {
			t.Fatal("accepted blob", bad)
		}
	}
	completion, err := core.ParseCompletionID("subagent:child:1:1790000000000")
	if err != nil || completion.String() != "subagent:child:1:1790000000000" || completion.BlockID().String() != completion.String() {
		t.Fatal("completion", err)
	}
	for _, bad := range []string{"subagent:child", "subagent::5", "subagent:child:", "subagent:child:x5", "subagent:child:+5", "agent:child:5", "subagent:child:9007199254740992"} {
		if _, err := core.ParseCompletionID(bad); err == nil {
			t.Fatal("accepted completion", bad)
		}
	}
}

func TestProviderStoredData(t *testing.T) {
	catalog, err := core.Decode[core.ProviderModelList](fixture(t, "catalog"))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := core.Encode(catalog)
	if err != nil {
		t.Fatal(err)
	}
	sameJSON(t, encoded, fixture(t, "catalog"))
	quota, err := core.Decode[core.QuotaSnapshot](fixture(t, "snapshot"))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err = core.Encode(quota)
	if err != nil {
		t.Fatal(err)
	}
	sameJSON(t, encoded, fixture(t, "snapshot"))
	for _, test := range []struct {
		file, path string
		value      any
		remove     bool
	}{
		{"catalog", "/models/0/providerId", "vendor", false},
		{"catalog", "/models/0/description", nil, true},
		{"catalog", "/models/0/outputLimit", 0, false},
		{"snapshot", "/windows/0/usedPercent", 100.5, false},
		{"snapshot", "/source", "cache", false},
		{"snapshot", "/raw", map[string]any{"plan_type": "pro"}, false},
	} {
		var value any
		if err := json.Unmarshal(fixture(t, test.file), &value); err != nil {
			t.Fatal(err)
		}
		mutate(t, value, test.path, test.value, test.remove)
		var err error
		if test.file == "catalog" {
			_, err = core.Decode[core.ProviderModelList](document(t, value))
		} else {
			_, err = core.Decode[core.QuotaSnapshot](document(t, value))
		}
		if err == nil {
			t.Fatal("accepted", test.path)
		}
	}
}

func TestDecodeCategoriesAndFieldPaths(t *testing.T) {
	for _, test := range []struct {
		data string
		kind core.DecodeErrorKind
	}{
		{`{"type":"text","id":`, core.DecodeSyntax},
		{`{"type":"text","id":7}`, core.DecodeShape},
		{`{"type":"missing"}`, core.DecodeShape},
		{`{"type":"text","type":"text"}`, core.DecodeShape},
		{`{"inputTokens":9007199254740992,"outputTokens":0,"cacheReadTokens":0,"cacheWriteTokens":0}`, core.DecodeInvalid},
	} {
		var err error
		if test.kind == core.DecodeInvalid {
			_, err = core.Decode[core.TokenUsage]([]byte(test.data))
		} else {
			_, err = core.Decode[core.Block]([]byte(test.data))
		}
		var refusal *core.DecodeError
		if !errors.As(err, &refusal) || refusal.Kind != test.kind {
			t.Fatalf("%s: %v", test.data, err)
		}
	}
	for _, value := range blocks(t) {
		if value["id"] != "u1" {
			continue
		}
		mutate(t, value, "/content/6/name", "", false)
		_, err := core.Decode[core.Block](document(t, value))
		if err == nil || !strings.Contains(err.Error(), "content[6].name") {
			t.Fatal("missing field path", err)
		}
	}
}

func TestTimestampRangeAndForkableDefault(t *testing.T) {
	for _, ms := range []int64{-377705023201000, -1, 0, 253402207200999} {
		stamp, err := core.TimestampFromMillisecond(ms)
		if err != nil {
			t.Fatal(err)
		}
		encoded, err := core.Encode(stamp)
		if err != nil {
			t.Fatal(err)
		}
		got, err := core.Decode[core.Timestamp](encoded)
		if err != nil || got != stamp {
			t.Fatalf("time %s: %v", encoded, err)
		}
	}
	for _, ms := range []int64{-377705023201001, 253402207201000} {
		if _, err := core.TimestampFromMillisecond(ms); err == nil {
			t.Fatal("accepted time outside range")
		}
	}
	if got := core.TruncateTimestamp(time.Unix(-1, 999500000)); got.Millisecond() != 0 {
		t.Fatal("negative truncation", got)
	}
	for _, value := range blocks(t) {
		if value["type"] != "text" {
			continue
		}
		value["forkable"] = false
		decoded, err := core.Decode[core.Block](document(t, value))
		if err != nil {
			t.Fatal(err)
		}
		encoded, err := core.Encode(decoded)
		if err != nil {
			t.Fatal(err)
		}
		delete(value, "forkable")
		sameJSON(t, encoded, document(t, value))
		return
	}
	t.Fatal("missing text fixture")
}
