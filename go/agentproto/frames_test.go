package agentproto

import (
	"encoding/json"
	"errors"
	"os"
	"reflect"
	"strconv"
	"strings"
	"testing"
)

// These fixture scenarios exercise the WebSocket boundary without I/O beyond
// reading the shared Rust fixtures; their expected cost is under one second.
func TestFrames(t *testing.T) {
	for _, direction := range []string{"client", "server"} {
		t.Run(direction, func(t *testing.T) {
			raw, err := os.ReadFile("testdata/" + direction + "-frames.json")
			if err != nil {
				t.Fatal(err)
			}
			var frames []json.RawMessage
			if err = json.Unmarshal(raw, &frames); err != nil {
				t.Fatal(err)
			}
			for _, raw := range frames {
				var expected map[string]any
				if err = json.Unmarshal(raw, &expected); err != nil {
					t.Fatal(err)
				}
				t.Run(expected["type"].(string), func(t *testing.T) {
					var encoded []byte
					if direction == "client" {
						frame, err := DecodeClientFrame(raw)
						if err != nil {
							t.Fatal(err)
						}
						if string(frame.(interface{ Kind() ClientFrameKind }).Kind()) != expected["type"] {
							t.Fatal("wrong frame kind")
						}
						encoded, err = Encode(frame)
						if err != nil {
							t.Fatal(err)
						}
					} else {
						frame, err := Decode[ServerFrame](raw)
						if err != nil {
							t.Fatal(err)
						}
						encoded, err = Encode(frame)
						if err != nil {
							t.Fatal(err)
						}
					}
					var actual any
					if err := json.Unmarshal(encoded, &actual); err != nil {
						t.Fatal(err)
					}
					if !reflect.DeepEqual(actual, expected) {
						t.Fatalf("wire shape changed: %s", encoded)
					}
				})
			}
		})
	}
}

func TestClientMutations(t *testing.T) {
	raw, err := os.ReadFile("testdata/client-frames.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures []map[string]any
	if err = json.Unmarshal(raw, &fixtures); err != nil {
		t.Fatal(err)
	}
	raw, err = os.ReadFile("testdata/client-frames-mutations.json")
	if err != nil {
		t.Fatal(err)
	}
	var table map[string][]struct {
		Why     string
		Fixture string
		Value   json.RawMessage
		Remove  []string
		Set     map[string]any
	}
	if err = json.Unmarshal(raw, &table); err != nil {
		t.Fatal(err)
	}
	for expectation, cases := range table {
		for _, c := range cases {
			t.Run(expectation+"/"+c.Why, func(t *testing.T) {
				raw := c.Value
				if raw == nil {
					for _, f := range fixtures {
						if f["type"] == c.Fixture {
							raw, err = json.Marshal(f)
							break
						}
					}
					if err != nil {
						t.Fatal(err)
					}
					var frame any
					if err = json.Unmarshal(raw, &frame); err != nil {
						t.Fatal(err)
					}
					for _, path := range c.Remove {
						mutateFrame(t, frame, path, nil, true)
					}
					for path, value := range c.Set {
						mutateFrame(t, frame, path, value, false)
					}
					raw, err = json.Marshal(frame)
					if err != nil {
						t.Fatal(err)
					}
				}
				_, err := DecodeClientFrame(raw)
				if expectation == "accepted" && err != nil {
					t.Fatal(err)
				}
				if expectation == "refused" {
					var frameError *FrameError
					if !errors.As(err, &frameError) || frameError.NotJSON {
						t.Fatalf("expected invalid frame, got %v", err)
					}
				}
			})
		}
	}
}

// mutateFrame applies the Rust fixture's JSON pointer to one frame.
func mutateFrame(t *testing.T, frame any, path string, value any, remove bool) {
	t.Helper()
	parts := strings.Split(strings.TrimPrefix(path, "/"), "/")
	for i, key := range parts {
		last := i == len(parts)-1
		switch current := frame.(type) {
		case map[string]any:
			if last {
				if remove {
					delete(current, key)
				} else {
					current[key] = value
				}
				return
			}
			frame = current[key]
		case []any:
			index, err := strconv.Atoi(key)
			if err != nil {
				t.Fatal(err)
			}
			if last {
				current[index] = value
				return
			}
			frame = current[index]
		default:
			t.Fatalf("invalid fixture pointer %s", path)
		}
	}
}

func TestFrameErrorCategoryAndForwardFields(t *testing.T) {
	for _, input := range []string{"not json", `{"type":`, ""} {
		_, err := DecodeClientFrame([]byte(input))
		var frameError *FrameError
		if !errors.As(err, &frameError) || !frameError.NotJSON {
			t.Fatalf("expected syntax error: %v", err)
		}
	}
	frame, err := Decode[ServerFrame]([]byte(`{"type":"phase","phase":"idle","since":"2026-09-21T14:13:20.000Z"}`))
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := frame.(ServerFramePhase); !ok {
		t.Fatalf("wrong frame: %T", frame)
	}
}
