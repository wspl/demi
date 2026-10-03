package core_test

import (
	"encoding/json"
	"os"
	"reflect"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

const fixtureDir = "testdata/"

// This boundary corpus protects stored transcript compatibility; it has no IO
// beyond local fixtures, no processes or timers, and a one-second test budget.
func TestBlockCorpus(t *testing.T) {
	data, err := os.ReadFile(fixtureDir + "blocks.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures []json.RawMessage
	if err := json.Unmarshal(data, &fixtures); err != nil {
		t.Fatal(err)
	}
	kinds := map[string]bool{}
	for _, fixture := range fixtures {
		var holder core.BlockJSON
		err := json.Unmarshal(fixture, &holder)
		block := holder.Value
		if err != nil {
			t.Fatalf("decode %s: %v", fixture, err)
		}
		if err := core.ValidateBlock(block); err != nil {
			t.Fatal(err)
		}
		encoded, err := contract.EncodeJSON(holder)
		if err != nil {
			t.Fatal(err)
		}
		var tag struct {
			Type      string              `json:"type"`
			ID        core.BlockID        `json:"id"`
			CreatedAt core.Timestamp      `json:"createdAt"`
			Model     core.ModelSelection `json:"model"`
		}
		if err := json.Unmarshal(fixture, &tag); err != nil {
			t.Fatal(err)
		}
		kinds[tag.Type] = true
		if block.ID() != tag.ID || block.CreatedAt() != tag.CreatedAt || !reflect.DeepEqual(block.Model(), tag.Model) {
			t.Fatalf("%s: block accessors do not expose stored metadata", tag.ID)
		}
		if block.IsEditable() != (tag.Type == "user") {
			t.Fatalf("%s: only user blocks are editable", tag.ID)
		}
		var left, right any
		if err := json.Unmarshal(fixture, &left); err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(encoded, &right); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(left, right) {
			t.Fatalf("roundtrip mismatch\n%s\n%s", fixture, encoded)
		}
	}
	for _, kind := range []string{
		"user",
		"context",
		"wakeup",
		"steer",
		"agent_message",
		"resume",
		"abort",
		"thinking",
		"redacted_thinking",
		"text",
		"tool_call",
		"response",
		"error",
		"compaction_boundary",
		"compaction_marker",
	} {
		if !kinds[kind] {
			t.Fatalf("missing %s fixture", kind)
		}
	}
	table, err := os.ReadFile(fixtureDir + "blocks-mutations.json")
	if err != nil {
		t.Fatal(err)
	}
	var cases struct {
		Refused []struct {
			Why           string         `json:"why"`
			Fixture       string         `json:"fixture"`
			Remove        []string       `json:"remove"`
			Set           map[string]any `json:"set"`
			WebAppRefuses bool           `json:"webAppRefuses"`
		} `json:"refused"`
	}
	if err := json.Unmarshal(table, &cases); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range cases.Refused {
		t.Run(scenario.Why, func(t *testing.T) {
			value := fixtureByID(t, fixtures, scenario.Fixture)
			for _, pointer := range scenario.Remove {
				mutate(t, value, pointer, nil, true)
			}
			for pointer, replacement := range scenario.Set {
				mutate(t, value, pointer, replacement, false)
			}
			encoded, err := contract.EncodeJSON(value)
			if err != nil {
				t.Fatal(err)
			}
			_, err = core.DecodeBlock(encoded)
			if err == nil {
				t.Fatalf("accepted: %s", encoded)
			}
		})
	}
	t.Logf("blocks=%d semantic_equal=%d refused=%d", len(fixtures), len(fixtures), len(cases.Refused))
}

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
