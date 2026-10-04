package main

import (
	"encoding/json"
	"os"
	"reflect"
	"strconv"
	"strings"
	"testing"

	contracts "github.com/wspl/demi/tools/contractgen/testdata/blocks"
)

const fixtureDir = "../../internal/core/testdata/"

// fixtureByID returns the fixture block with this id, which a refusal case mutates.
func fixtureByID(t *testing.T, fixtures []json.RawMessage, id string) map[string]any {
	t.Helper()
	for _, raw := range fixtures {
		var value map[string]any
		if err := json.Unmarshal(raw, &value); err != nil {
			t.Fatal(err)
		}
		if value["id"] == id {
			return value
		}
	}
	t.Fatalf("missing fixture %s", id)
	return nil
}

// mutate applies the corpus's object removal or object/array replacement.
func mutate(t *testing.T, root map[string]any, pointer string, value any, remove bool) {
	t.Helper()
	parts := strings.Split(strings.TrimPrefix(pointer, "/"), "/")
	var current any = root
	for i, key := range parts {
		key = strings.ReplaceAll(strings.ReplaceAll(key, "~1", "/"), "~0", "~")
		last := i == len(parts)-1
		switch target := current.(type) {
		case map[string]any:
			if last {
				if remove {
					if _, ok := target[key]; !ok {
						t.Fatal("remove missing field")
					}
					delete(target, key)
				} else {
					target[key] = value
				}
				return
			}
			current = target[key]
		case []any:
			index, err := strconv.Atoi(key)
			if err != nil || index < 0 || index >= len(target) {
				t.Fatal("bad array pointer")
			}
			if last {
				if remove {
					t.Fatal("the corpus removes object fields only")
				}
				target[index] = value
				return
			}
			current = target[index]
		default:
			t.Fatalf("invalid fixture pointer %s", pointer)
		}
	}
}

// The generated transcript decoder's edge cases, on blocks of the stored
// corpus that internal/core tests against the real types. In-memory fixture
// reads only; budget one second.
func TestBoundaryEdges(t *testing.T) {
	raw, err := os.ReadFile(fixtureDir + "blocks.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures []json.RawMessage
	if err := json.Unmarshal(raw, &fixtures); err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name, id, pointer string
		value             any
		accepted          bool
	}{
		{"64 astral scalars", "c1", "/source", strings.Repeat("😀", 64), true},
		{"65 astral scalars", "c1", "/source", strings.Repeat("😀", 65), false},
		{"optional empty is present", "u1", "/content/6/snippet", "", true},
		{"required string null", "u1", "/content/0/text", nil, false},
		{"required array null", "u1", "/content", nil, false},
		{"required object null", "u1", "/model", nil, false},
		{"canonical milliseconds", "u1", "/createdAt", "2026-09-21T14:13:20.123Z", true},
		{"comma fraction", "u1", "/createdAt", "2026-09-21T14:13:20,000Z", false},
		{"no fractional digits", "u1", "/createdAt", "2026-09-21T14:13:20Z", false},
		{"offset time", "u1", "/createdAt", "2026-09-21T14:13:20.000+00:00", false},
		{"invalid calendar", "u1", "/createdAt", "2026-02-30T14:13:20.000Z", false},
		{"unsafe usage", "re1", "/usage/inputTokens", uint64(9007199254740992), false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			value := fixtureByID(t, fixtures, tc.id)
			mutate(t, value, tc.pointer, tc.value, false)
			data, err := json.Marshal(value)
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := contracts.DecodeBlock(data)
			if (err == nil) != tc.accepted {
				t.Fatalf("accepted=%v err=%v", tc.accepted, err)
			}
			if tc.accepted {
				encoded, err := json.Marshal(decoded)
				if err != nil {
					t.Fatal(err)
				}
				var before, after any
				if err := json.Unmarshal(data, &before); err != nil {
					t.Fatal(err)
				}
				if err := json.Unmarshal(encoded, &after); err != nil {
					t.Fatal(err)
				}
				if !reflect.DeepEqual(before, after) {
					t.Fatalf("accepted edge changed on encode: %s -> %s", data, encoded)
				}

			}
		})
	}
	// A refusal inside a nested value names the path to the refused field.
	attachment := fixtureByID(t, fixtures, "u1")
	mutate(t, attachment, "/content/6/name", "", false)
	data, err := json.Marshal(attachment)
	if err != nil {
		t.Fatal(err)
	}
	_, err = contracts.DecodeBlock(data)
	if err == nil {
		t.Fatal("accepted an attachment without a name")
	}
	for _, part := range []string{"content", "[6]", "name"} {
		if !strings.Contains(err.Error(), part) {
			t.Fatalf("refusal %q does not name %s", err, part)
		}
	}
	for _, data := range []string{`{"type":"resume","type":"resume"}`, `{"type":"text","text":"\ud800"}`} {
		if _, err := contracts.DecodeBlock([]byte(data)); err == nil {
			t.Fatalf("accepted malformed input %s", data)
		}
	}
}
