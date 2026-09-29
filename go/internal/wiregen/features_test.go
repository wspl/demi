package wiregen

import (
	"bytes"
	"encoding/json/jsontext"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/internal/wiregen/exporttest"
	"github.com/wspl/demi/go/internal/wiregen/featuretest"
)

// These contract scenarios are in-process, without timers; their nominal cost is under one second.
func TestOptionalNullsAndFalseOmission(t *testing.T) {
	for _, packed := range []bool{false, true} {
		for _, scenario := range []struct{ input, output, state string }{
			{`{}`, `{}`, "absent"},
			{`{"usage":null,"forkable":false}`, `{}`, "absent"},
			{`{"setting":null}`, `{"setting":null}`, "null"},
			{`{"usage":{"tokens":1},"setting":"on","forkable":true}`, `{"usage":{"tokens":1},"setting":"on","forkable":true}`, "value"},
		} {
			var value featuretest.Patch
			var output []byte
			var err error
			if packed {
				value, err = featuretest.DecodePatchMsgpack(packJSON(t, scenario.input))
			} else {
				value, err = featuretest.DecodePatchJSON([]byte(scenario.input))
			}
			if err != nil {
				t.Fatal(err)
			}
			switch scenario.state {
			case "absent":
				if value.Setting != nil {
					t.Fatal("absence was not preserved")
				}
			case "null":
				if value.Setting == nil || *value.Setting != nil {
					t.Fatal("null was not preserved")
				}
			case "value":
				if value.Setting == nil || *value.Setting == nil || **value.Setting != "on" {
					t.Fatal("value was not preserved")
				}
			}
			if packed {
				output, err = value.MarshalMsgpack()
				if err == nil {
					output, err = wire.MPJSON(output)
				}
			} else {
				output, err = featuretest.EncodePatchJSON(value)
			}
			if err != nil || string(output) != scenario.output {
				t.Fatalf("packed=%v: %s, %v; want %s", packed, output, err, scenario.output)
			}
		}
		for _, raw := range []string{`{"setting":""}`, `{"setting":1}`, `{"usage":{"tokens":0}}`, `{"usage":{}}`, `{"forkable":null}`, `{"forkable":1}`} {
			var err error
			if packed {
				_, err = featuretest.DecodePatchMsgpack(packJSON(t, raw))
			} else {
				_, err = featuretest.DecodePatchJSON([]byte(raw))
			}
			if err == nil {
				t.Fatalf("packed=%v accepted %s", packed, raw)
			}
		}
	}
}

func TestRetainedMembersReplayAndCannotShadow(t *testing.T) {
	raw := `{"id":"r","encrypted_content":"opaque-secret","nested":{"more":[1,null,true]}}`
	for _, packed := range []bool{false, true} {
		var value featuretest.Reasoning
		var encoded []byte
		var err error
		if packed {
			value, err = featuretest.DecodeReasoningMsgpack(packJSON(t, raw))
		} else {
			value, err = featuretest.DecodeReasoningJSON([]byte(raw))
		}
		if err != nil {
			t.Fatal(err)
		}
		if string(value.Extra["encrypted_content"]) != `"opaque-secret"` || string(value.Extra["nested"]) != `{"more":[1,null,true]}` {
			t.Fatalf("lost unknown members: %v", value.Extra)
		}
		if packed {
			encoded, err = value.MarshalMsgpack()
			if err == nil {
				encoded, err = wire.MPJSON(encoded)
			}
		} else {
			encoded, err = featuretest.EncodeReasoningJSON(value)
		}
		if err != nil || string(encoded) != raw {
			t.Fatalf("packed=%v: %s %v", packed, encoded, err)
		}
		value.Extra["hidden"] = jsontext.Value(`true`)
		if packed {
			_, err = value.MarshalMsgpack()
		} else {
			_, err = featuretest.EncodeReasoningJSON(value)
		}
		if err == nil || !strings.Contains(err.Error(), "hidden") {
			t.Fatalf("shadowed an omitted known field: %v", err)
		}
	}
	if _, err := featuretest.DecodeReasoningJSON([]byte(`{"id":"r","x":1,"x":2}`)); err == nil {
		t.Fatal("accepted duplicate unknown member")
	}
	if _, err := featuretest.DecodeReasoningMsgpack([]byte{0x83, 0xa2, 'i', 'd', 0xa1, 'r', 0xa1, 'x', 1, 0xa1, 'x', 2}); err == nil {
		t.Fatal("accepted duplicate unknown MessagePack member")
	}
}

func TestEmbeddedVariantRunsInheritedChecks(t *testing.T) {
	for _, packed := range []bool{false, true} {
		for _, scenario := range []struct {
			raw   string
			valid bool
		}{
			{`{"type":"changed","low":1,"high":2}`, true},
			{`{"type":"changed","low":-1,"high":2}`, false},
			{`{"type":"changed","low":2,"high":1}`, false},
		} {
			var value featuretest.Event
			var err error
			if packed {
				value, err = featuretest.DecodeEventMsgpack(packJSON(t, scenario.raw))
			} else {
				value, err = featuretest.DecodeEventJSON([]byte(scenario.raw))
			}
			if (err == nil) != scenario.valid {
				t.Fatalf("packed=%v: %s: %v", packed, scenario.raw, err)
			}
			if !scenario.valid {
				continue
			}
			var output []byte
			if packed {
				output, err = featuretest.EncodeEventMsgpack(value)
				if err == nil {
					output, err = wire.MPJSON(output)
				}
			} else {
				output, err = featuretest.EncodeEventJSON(value)
			}
			if err != nil || string(output) != scenario.raw {
				t.Fatalf("wrong flattened encoding: %s %v", output, err)
			}
		}
	}
	invalid := featuretest.Changed{BoundInfo: featuretest.BoundInfo{Bounds: featuretest.Bounds{Low: 3, High: 1}}}
	if _, err := featuretest.EncodeEventJSON(invalid); err == nil {
		t.Fatal("encoded an invalid embedded check")
	}
}

func TestStandaloneAndForeignOwnerEntryPoints(t *testing.T) {
	if _, err := exporttest.DecodeModeJSON([]byte(`"other"`)); err == nil {
		t.Fatal("standalone enum accepted unknown value")
	}
	if _, err := exporttest.DecodeModeJSON([]byte(`"read" true`)); err == nil {
		t.Fatal("standalone enum accepted trailing input")
	}
	mode, err := exporttest.DecodeModeJSON([]byte(`"read"`))
	if err != nil {
		t.Fatal(err)
	}
	if raw, err := exporttest.EncodeModeJSON(mode); err != nil || string(raw) != `"read"` {
		t.Fatalf("enum encoding: %s %v", raw, err)
	}
	if _, err := exporttest.EncodeModeJSON(exporttest.Mode("other")); err == nil {
		t.Fatal("encoded invalid enum")
	}
	raw := `{"mode":"read","choice":{"kind":"limited","count":1},"tab":{"id":"t1","title":"tab","url":"https://example.test","createdBy":{"kind":"user"}}}`
	value, err := featuretest.DecodeForeignJSON([]byte(raw))
	if err != nil {
		t.Fatal(err)
	}
	if output, err := featuretest.EncodeForeignJSON(value); err != nil || string(output) != raw {
		t.Fatalf("foreign encoding: %s %v", output, err)
	}
	for _, change := range [][2]string{{`"read"`, `"other"`}, {`"count":1`, `"count":0`}, {`"id":"t1"`, `"id":"invalid"`}} {
		_, err := featuretest.DecodeForeignJSON([]byte(strings.Replace(raw, change[0], change[1], 1)))
		if err == nil {
			t.Fatalf("foreign owner failed to check %v", change)
		}
	}
}

func TestNestedSyntaxKeepsTypedCauseAndPath(t *testing.T) {
	_, err := featuretest.DecodePatchJSON([]byte(`{"usage":{"tokens":1e+SECRET}}`))
	var syntax *jsontext.SyntacticError
	var invalid *wire.InvalidError
	if !errors.As(err, &syntax) || !errors.As(err, &invalid) {
		t.Fatalf("lost syntax cause: %v", err)
	}
	if invalid.Path != "usage.tokens" || strings.Contains(err.Error(), "SECRET") {
		t.Fatalf("wrong path or leaked value: %v", err)
	}
}

// packJSON prepares protocol input using the shared JSON-valued MessagePack codec.
func packJSON(t *testing.T, raw string) []byte {
	t.Helper()
	data, err := wire.JSONMsgpack(jsontext.Value(raw))
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func TestBackendFeatureGoldenAndEmitterModel(t *testing.T) {
	for _, dir := range []string{"exporttest", "featuretest"} {
		pkg, err := Load(dir)
		if err != nil {
			t.Fatal(err)
		}
		files, err := pkg.GenerateGo()
		if err != nil {
			t.Fatal(err)
		}
		for name, data := range files {
			compare(t, filepath.Join("testdata", "backend", dir, name+".golden"), data)
		}
		if dir != "featuretest" {
			continue
		}
		patch := pkg.Structs["Patch"]
		if !patch.Fields[0].NullAsAbsent || !patch.Fields[1].TriState || patch.Fields[2].Required {
			t.Fatal("schema emitters lack field presence")
		}
		if pkg.Structs["Reasoning"].Unknown == nil || len(pkg.Structs["Changed"].EmbedChecks) != 1 {
			t.Fatal("schema emitters lack flatten metadata")
		}
		scalar := pkg.Structs["Timed"].Fields[0].Type
		if scalar.WireKind != KindString || scalar.Format != "date-time" || !pkg.Exported["Timed"] {
			t.Fatal("schema emitters lack opaque or export metadata")
		}
	}
	// The generated owner entry points are additional output; its old codec stays byte-identical.
	pkg, err := Load("../../builtinproto")
	if err != nil {
		t.Fatal(err)
	}
	files, err := pkg.GenerateGo()
	if err != nil {
		t.Fatal(err)
	}
	old, err := os.ReadFile("../../builtinproto/browser_wire.go")
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(old, files["browser_wire.go"]) {
		t.Fatal("export changed existing JSON codec")
	}
}

func TestBackendFeatureDeclarationRefusals(t *testing.T) {
	for _, body := range []string{
		"//demi:wire\ntype T struct{A string `json:\"a,omitzero\" check:\"nullabsent\"`}",
		"//demi:wire\ntype T struct{A *string `json:\"a\" check:\"nullabsent\"`}",
		"//demi:wire\ntype T struct{A **string `json:\"a,omitzero\"`}",
		"//demi:wire\ntype T struct{A ***string `json:\"a,omitzero\" check:\"nullable\"`}",
		"//demi:wire open\ntype T struct{Extra map[string]jsontext.Value `json:\",inline\"`}",
		"//demi:wire\ntype T struct{A map[string]jsontext.Value `json:\",inline\"`; B map[string]jsontext.Value `json:\",inline\"`}",
		"//demi:opaque number\ntype T struct{}",
		"//demi:opaque string format=\ntype T struct{}",
		"//demi:wire\n//demi:export bad\ntype T struct{}",
	} {
		if _, err := LoadSource(map[string][]byte{"p.go": []byte("package p\nimport \"encoding/json/jsontext\"\n" + body)}); err == nil {
			t.Fatalf("accepted %s", body)
		}
	}
}

// Cost: in-memory codecs only; no timers or external services.
func TestRetainedAdjacentVariantAndInlineFalse(t *testing.T) {
	raw := `{"kind":"reasoning","data":{"id":"r","opaque":[1,null]}}`
	value, err := featuretest.DecodeEnvelopeJSON([]byte(raw))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := featuretest.EncodeEnvelopeJSON(value)
	if err != nil || string(encoded) != raw {
		t.Fatalf("inline false and retained variant: %s %v", encoded, err)
	}
	packed, err := featuretest.EncodeReplayMsgpack(value.Replay)
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := featuretest.DecodeReplayMsgpack(packed)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err = featuretest.EncodeReplayJSON(decoded)
	if err != nil || string(encoded) != raw {
		t.Fatalf("retained adjacent MessagePack: %s %v", encoded, err)
	}
}
