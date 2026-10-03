package core_test

import (
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

// Stored provider fixtures and mutations pin restoration; budget is one second.
func TestStoredProviderData(t *testing.T) {
	for _, scenario := range []struct {
		file      string
		decode    func([]byte) (any, error)
		mutations []struct {
			pointer string
			value   any
			remove  bool
		}
	}{
		{"catalog", func(b []byte) (any, error) { return core.DecodeProviderModelList(b) }, []struct {
			pointer string
			value   any
			remove  bool
		}{
			{"/models/0/providerId", "anthropic", false}, {
				"/models/0/description",
				nil,
				true,
			}, {
				"/models/0/outputLimit",
				0,
				false,
			},
		}},
		{"snapshot", func(b []byte) (any, error) { return core.DecodeQuotaSnapshot(b) }, []struct {
			pointer string
			value   any
			remove  bool
		}{
			{"/windows/0/usedPercent", 100.5, false}, {
				"/source",
				"cache",
				false,
			}, {
				"/raw",
				map[string]any{
					"plan_type": "pro",
				},
				false,
			},
		}},
	} {
		t.Run(scenario.file, func(t *testing.T) {
			data, err := os.ReadFile("testdata/" + scenario.file + ".json")
			if err != nil {
				t.Fatal(err)
			}
			decoded, err := scenario.decode(data)
			if err != nil {
				t.Fatal(err)
			}
			encoded, err := contract.EncodeJSON(decoded)
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
				t.Fatalf("wire shape changed: %s", encoded)
			}
			for _, mutation := range scenario.mutations {
				var value map[string]any
				if err := json.Unmarshal(data, &value); err != nil {
					t.Fatal(err)
				}
				mutate(t, value, mutation.pointer, mutation.value, mutation.remove)
				raw, err := contract.EncodeJSON(value)
				if err != nil {
					t.Fatal(err)
				}
				if _, err := scenario.decode(raw); err == nil {
					t.Errorf("accepted %s", raw)
				}
			}
		})
	}
}

func TestRuleErrorsNameFields(t *testing.T) {
	data, err := os.ReadFile("testdata/blocks.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures []json.RawMessage
	if err := json.Unmarshal(data, &fixtures); err != nil {
		t.Fatal(err)
	}
	for _, scenario := range []struct {
		id, path string
		value    any
		parts    []string
	}{
		{"u1", "/content/6/name", "", []string{"content[6].name"}},
		{"m1", "/id", "m2", []string{"must be the message's id, m1"}},
	} {
		t.Run(scenario.id, func(t *testing.T) {
			value := fixtureByID(t, fixtures, scenario.id)
			mutate(t, value, scenario.path, scenario.value, false)
			raw, err := contract.EncodeJSON(value)
			if err != nil {
				t.Fatal(err)
			}
			_, err = core.DecodeBlock(raw)
			var field *contract.Error
			if err == nil {
				t.Fatal("accepted invalid block")
			}
			// Accepted normalization F4: paths use wire fields without Rust enum wrappers.
			if scenario.id == "u1" && (!errors.As(err, &field) || field.Path != "content[6].name") {
				t.Fatalf("expected field error: %v", err)
			}
			for _, part := range scenario.parts {
				if !strings.Contains(err.Error(), part) {
					t.Errorf("missing %q in %v", part, err)
				}
			}
		})
	}
}

func TestReceiveOnlyStatesTolerateNewFields(t *testing.T) {
	for _, scenario := range []struct {
		raw    string
		decode func([]byte) error
	}{
		{`{"status":"authenticated","accountLabel":"A","future":true}`, func(b []byte) error {
			_, err := core.DecodeAuthState(b)
			return err
		}},
		{`{"status":"ready","future":true}`, func(b []byte) error {
			_, err := core.DecodeRuntimeState(b)
			return err
		}},
	} {
		if err := scenario.decode([]byte(scenario.raw)); err != nil {
			t.Error(err)
		}
	}
	if _, err := core.DecodeAuthState([]byte(`{"status":"authenticated","accountLabel":null}`)); err == nil {
		t.Fatal("optional null accepted")
	}
}

// Pending steers are receive-only page state, so new fields must be tolerated.
// Cost: one local block fixture and generated decoding, below one millisecond.
func TestPendingSteerToleratesUnknownFields(t *testing.T) {
	data, err := os.ReadFile("testdata/blocks.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures []json.RawMessage
	if err := json.Unmarshal(data, &fixtures); err != nil {
		t.Fatal(err)
	}
	block, err := core.DecodeBlock(fixtures[0])
	if err != nil {
		t.Fatal(err)
	}
	want := core.PendingSteer{ID: "s1", TurnID: "t1", Model: block.Model(), Content: []core.UserContentBlock{}}
	data, err = contract.EncodeJSON(want)
	if err != nil {
		t.Fatal(err)
	}
	data = append(data[:len(data)-1], []byte(`,"future":true}`)...)
	got, err := core.DecodePendingSteer(data)
	if err != nil || !reflect.DeepEqual(got, want) {
		t.Fatalf("pending steer: %#v %v", got, err)
	}
}
