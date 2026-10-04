package framewire_test

import (
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// The wire corpus in testdata protects the frames consumed by the web app and
// backend. These tests use local files only, with a one-second suite budget.
func TestClientFrames(t *testing.T) {
	fixtures := readFixtures(t, "client-frames.json")
	kinds := map[framewire.ClientFrameKind]bool{}
	for _, raw := range fixtures {
		frame, err := framewire.DecodeClientFrame(raw)
		if err != nil {
			t.Fatalf("%s: %v", raw, err)
		}
		var tag struct {
			Type string `json:"type"`
		}
		if err := json.Unmarshal(raw, &tag); err != nil {
			t.Fatal(err)
		}
		if string(frame.Kind()) != tag.Type {
			t.Fatalf("kind %s != %s", frame.Kind(), tag.Type)
		}
		kinds[frame.Kind()] = true
		roundTrip(t, raw, frame)
	}
	if len(kinds) != 19 {
		t.Fatalf("got %d frame kinds", len(kinds))
	}
}

func TestServerFrames(t *testing.T) {
	fixtures := readFixtures(t, "server-frames.json")
	kinds := map[string]bool{}
	for _, raw := range fixtures {
		frame, err := framewire.DecodeServerFrame(raw)
		if err != nil {
			t.Fatalf("%s: %v", raw, err)
		}
		var tag struct {
			Type string `json:"type"`
		}
		if err := json.Unmarshal(raw, &tag); err != nil {
			t.Fatal(err)
		}
		kinds[tag.Type] = true
		roundTrip(t, raw, frame)
	}
	if len(kinds) != 19 {
		t.Fatalf("got %d frame kinds", len(kinds))
	}
}

func TestClientMutations(t *testing.T) {
	var table map[string][]struct {
		Why     string          `json:"why"`
		Fixture string          `json:"fixture"`
		Value   json.RawMessage `json:"value"`
		Remove  []string        `json:"remove"`
		Set     map[string]any  `json:"set"`
	}
	data, err := os.ReadFile("testdata/client-frames-mutations.json")
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &table); err != nil {
		t.Fatal(err)
	}
	fixtures := readFixtures(t, "client-frames.json")
	for category, cases := range table {
		for _, scenario := range cases {
			t.Run(category+"/"+scenario.Why, func(t *testing.T) {
				raw := scenario.Value
				if raw == nil {
					value := fixture(t, fixtures, scenario.Fixture)
					for _, pointer := range scenario.Remove {
						mutate(t, value, pointer, nil, true)
					}
					for pointer, replacement := range scenario.Set {
						mutate(t, value, pointer, replacement, false)
					}
					raw, err = contract.EncodeJSON(value)
					if err != nil {
						t.Fatal(err)
					}
				}
				_, err := framewire.DecodeClientFrame(raw)
				if category == "accepted" {
					if err != nil {
						t.Fatal(err)
					}
				} else if !errors.Is(err, framewire.ErrInvalidFrame) {
					t.Fatalf("expected invalid frame, got %v", err)
				}
			})
		}
	}
}

func TestFrameErrorClassification(t *testing.T) {
	for _, text := range []string{
		"not json",
		`{"type":`,
		"",
		`{"type":"open"} {}`,
		`{"type":"shell_write","commandId":"cmd-1","stdin":"\ud800"}`,
	} {
		_, err := framewire.DecodeClientFrame([]byte(text))
		if !errors.Is(err, framewire.ErrNotJSON) {
			t.Fatalf("%q: %v", text, err)
		}
	}
	for _, text := range []string{`[1,2]`, `{"type":"nope"}`, `null`, `{"type":"open","type":"open"}`} {
		_, err := framewire.DecodeClientFrame([]byte(text))
		if !errors.Is(err, framewire.ErrInvalidFrame) {
			t.Fatalf("%q: %v", text, err)
		}
	}
	for _, scenario := range []struct {
		pointer string
		value   any
		path    string
	}{
		{"/content/2/fileName", "..", "content[2].fileName"},
		{"/content/0", map[string]any{"type": "attachment", "path": "/a"}, "content[0]"},
	} {
		value := fixture(t, readFixtures(t, "client-frames.json"), "send")
		mutate(t, value, scenario.pointer, scenario.value, false)
		raw, err := contract.EncodeJSON(value)
		if err != nil {
			t.Fatal(err)
		}
		_, err = framewire.DecodeClientFrame(raw)
		var field *contract.Error
		if !errors.As(err, &field) || field.Path != scenario.path {
			t.Fatalf("wanted path %s: %v", scenario.path, err)
		}
	}
}

func TestServerUnknownFields(t *testing.T) {
	frame, err := framewire.DecodeServerFrame(
		[]byte(`{"type":"phase","phase":"idle","since":"2026-09-21T14:13:20.000Z"}`),
	)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(frame, &framewire.PhaseFrame{Phase: core.SessionPhaseIdle}) {
		t.Fatalf("%#v", frame)
	}
}

// readFixtures reads the shared wire corpus without interpreting its frames.
func readFixtures(t *testing.T, name string) []json.RawMessage {
	t.Helper()
	data, err := os.ReadFile("testdata/" + name)
	if err != nil {
		t.Fatal(err)
	}
	var fixtures []json.RawMessage
	if err := json.Unmarshal(data, &fixtures); err != nil {
		t.Fatal(err)
	}
	return fixtures
}

// roundTrip checks that a decoded conversation frame retains its wire value.
func roundTrip(t *testing.T, raw []byte, frame any) {
	t.Helper()
	encoded, err := contract.EncodeJSON(frame)
	if err != nil {
		t.Fatal(err)
	}
	var before, after any
	if err := json.Unmarshal(raw, &before); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(encoded, &after); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(before, after) {
		t.Fatalf("roundtrip:\n%s\n%s", raw, encoded)
	}
}

// fixture selects a fresh conversation frame for a mutation scenario.
func fixture(t *testing.T, fixtures []json.RawMessage, kind string) map[string]any {
	t.Helper()
	for _, raw := range fixtures {
		var value map[string]any
		if err := json.Unmarshal(raw, &value); err != nil {
			t.Fatal(err)
		}
		if value["type"] == kind {
			return value
		}
	}
	t.Fatalf("missing %s frame", kind)
	return nil
}

// mutate applies the corpus's JSON pointers to a conversation frame.
func mutate(t *testing.T, value any, pointer string, replacement any, remove bool) {
	t.Helper()
	parts := strings.Split(strings.TrimPrefix(pointer, "/"), "/")
	for i, part := range parts {
		part = strings.ReplaceAll(strings.ReplaceAll(part, "~1", "/"), "~0", "~")
		last := i == len(parts)-1
		switch parent := value.(type) {
		case map[string]any:
			if last {
				if remove {
					delete(parent, part)
				} else {
					parent[part] = replacement
				}
				return
			}
			value = parent[part]
		case []any:
			index, err := strconv.Atoi(part)
			if err != nil || index < 0 || index >= len(parent) {
				t.Fatalf("invalid pointer %s", pointer)
			}
			if last {
				parent[index] = replacement
				return
			}
			value = parent[index]
		default:
			t.Fatalf("invalid pointer %s", pointer)
		}
	}
}

// TestNestingLimitClosesConnection protects the socket's syntax-error policy.
func TestNestingLimitClosesConnection(t *testing.T) {
	data := []byte(strings.Repeat("[", 128) + "0" + strings.Repeat("]", 128))
	_, err := framewire.DecodeClientFrame(data)
	if !errors.Is(err, framewire.ErrNotJSON) || !errors.Is(err, contract.ErrSyntax) {
		t.Fatalf("expected wrapped syntax error that closes the connection, got %v", err)
	}
}
