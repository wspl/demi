package main

import (
	"bytes"
	"encoding/json"
	"math"
	"os"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/tools/contractgen/testdata/browser"
)

// These browser boundaries use local generated codecs and captured manifest
// schemas. Budget: one second, no services, processes, or waits.
func TestBrowserSchemas(t *testing.T) {
	for name, schema := range map[string]func() json.RawMessage{
		"BrowserFailure":      browser.BrowserFailureJSONSchema,
		"NodeValue":           browser.NodeValueJSONSchema,
		"ErrorDetails":        browser.ErrorDetailsJSONSchema,
		"DialogInspectResult": browser.DialogInspectResultJSONSchema,
	} {
		t.Run(name, func(t *testing.T) {
			raw, err := os.ReadFile("testdata/browser/" + name + ".json")
			if err != nil {
				t.Fatal(err)
			}
			var want bytes.Buffer
			if err := json.Compact(&want, raw); err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(schema(), want.Bytes()) {
				t.Fatalf("schema differs:\ngot %s\nwant %s", schema(), raw)
			}
		})
	}
}

func TestBrowserScalarVariants(t *testing.T) {
	for _, text := range []string{`"hello <>&\u2028"`, `-12.5`, `0`} {
		value, err := browser.DecodeNodeValue([]byte(text))
		if err != nil {
			t.Fatal(err)
		}
		encoded, err := contract.EncodeJSON(value)
		if err != nil {
			t.Fatal(err)
		}
		next, err := browser.DecodeNodeValue(encoded)
		if err != nil || !reflect.DeepEqual(value, next) {
			t.Fatalf("round trip: %s %v", encoded, err)
		}
		packed, err := browser.EncodeNodeValueMsgpack(value)
		if err != nil {
			t.Fatal(err)
		}
		next, err = browser.DecodeNodeValueMsgpack(packed)
		if err != nil || !reflect.DeepEqual(value, next) {
			t.Fatalf("MessagePack round trip: %x %v", packed, err)
		}
	}
	for _, text := range []string{`null`, `true`, `{}`, `[]`, `1e999`, `"\ud800"`, `1 2`} {
		if _, err := browser.DecodeNodeValue([]byte(text)); err == nil {
			t.Fatalf("accepted %s", text)
		}
	}
	for _, test := range []struct {
		text string
		want browser.Scalar
	}{
		{`-1`, new(browser.First(-1))},
		{`-11`, new(browser.Second(-11))},
		{`1.5`, new(browser.Second(1.5))},
		{`true`, new(browser.Boolean(true))},
		{`{"value":"x"}`, &browser.Object{Value: "x"}},
	} {
		got, err := browser.DecodeScalar([]byte(test.text))
		if err != nil || !reflect.DeepEqual(got, test.want) {
			t.Fatalf("%s: got %T %v; want %T", test.text, got, err, test.want)
		}
		packed, err := contract.EncodeMsgpack(json.RawMessage(test.text))
		if err != nil {
			t.Fatal(err)
		}
		got, err = browser.DecodeScalarMsgpack(packed)
		if err != nil || !reflect.DeepEqual(got, test.want) {
			t.Fatalf("MessagePack %s: got %T %v; want %T", test.text, got, err, test.want)
		}
	}
	for _, value := range []browser.NodeValue{nil, (*browser.NodeValueText)(nil), new(browser.NodeValueNumber(math.Inf(1)))} {
		if _, err := contract.EncodeJSON(browser.NodeValueJSON{Value: value}); err == nil {
			t.Fatal("encoded invalid variant")
		}
	}
}

func TestBrowserOptionalFlatten(t *testing.T) {
	for _, test := range []struct {
		input, output string
		present       bool
	}{
		{`{}`, `{}`, false},
		{`{"url":"x"}`, `{"url":"x"}`, false},
		{`{"directory":"d","manifest":"m","files":[]}`, `{"directory":"d","manifest":"m","files":[]}`, true},
		{`{"directory":"d"}`, `{}`, false},
		{`{"directory":null,"manifest":"m","files":[]}`, `{}`, false},
		{`{"directory":"d","manifest":"m","files":null}`, `{}`, false},
		{`{"directory":"d","manifest":"m","files":[{}]}`, `{}`, false},
		{`{"unknown":1,"directory":"d","manifest":"m","files":[]}`, `{"directory":"d","manifest":"m","files":[]}`, true},
	} {
		t.Run(test.input, func(t *testing.T) {
			v, err := browser.DecodeErrorDetails([]byte(test.input))
			if err != nil {
				t.Fatal(err)
			}
			if (v.AssetsExportResult != nil) != test.present {
				t.Fatalf("presence: %+v", v)
			}
			got, err := contract.EncodeJSON(v)
			if err != nil || string(got) != test.output {
				t.Fatalf("got %s %v, want %s", got, err, test.output)
			}
			packed, err := contract.EncodeMsgpack(json.RawMessage(test.input))
			if err != nil {
				t.Fatal(err)
			}
			next, err := browser.DecodeErrorDetailsMsgpack(packed)
			if err != nil || !reflect.DeepEqual(v, next) {
				t.Fatalf("MessagePack: %+v %v", next, err)
			}
			packed, err = next.MarshalMsgpack()
			if err != nil {
				t.Fatal(err)
			}
			next, err = browser.DecodeErrorDetailsMsgpack(packed)
			if err != nil || !reflect.DeepEqual(v, next) {
				t.Fatalf("MessagePack round trip: %+v %v", next, err)
			}
		})
	}
	for _, text := range []string{`{"url":null}`, `{"directory":"a","directory":"b"}`, `{"directory":"\ud800"}`} {
		if _, err := browser.DecodeErrorDetails([]byte(text)); err == nil {
			t.Fatalf("accepted invalid parent: %s", text)
		}
	}
	v := browser.ErrorDetails{AssetsExportResult: &browser.AssetsExportResult{}}
	if _, err := contract.EncodeJSON(v); err == nil {
		t.Fatal("encoded invalid present export")
	}
	input := `{"before":"b","type":"alert","message":"m","after":"a"}`
	value, err := browser.DecodeOptional([]byte(input))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := contract.EncodeJSON(value)
	if err != nil || string(encoded) != input {
		t.Fatalf("flattened field order: %s %v", encoded, err)
	}
}

func TestBrowserNegativeBoundsAndNullable(t *testing.T) {
	for _, input := range []string{`{"deltaX":-10000,"deltaY":10000}`, `{"deltaX":0,"deltaY":-0.5}`} {
		if _, err := browser.DecodeWheel([]byte(input)); err != nil {
			t.Fatal(err)
		}
	}
	for _, input := range []string{`{"deltaX":-10000.1,"deltaY":0}`, `{"deltaX":0,"deltaY":10000.1}`} {
		if _, err := browser.DecodeWheel([]byte(input)); err == nil {
			t.Fatalf("accepted %s", input)
		}
	}
	for _, input := range []string{`{"dialog":null}`, `{"dialog":{"type":"prompt","message":"hello"}}`} {
		v, err := browser.DecodeDialogInspectResult([]byte(input))
		if err != nil {
			t.Fatal(err)
		}
		got, err := contract.EncodeJSON(v)
		if err != nil || string(got) != input {
			t.Fatalf("nullable round trip: %s %v", got, err)
		}
	}
	for _, input := range []string{`{}`, `{"dialog":{}}`, `{"dialog":false}`} {
		if _, err := browser.DecodeDialogInspectResult([]byte(input)); err == nil {
			t.Fatalf("accepted %s", input)
		}
	}
}

// All nullable shapes occurring in the reference manifest must keep schemars'
// exact representation, including its reference alternatives and annotations.
func TestManifestNullableShapes(t *testing.T) {
	raw, err := os.ReadFile("testdata/browser/nullable-manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	var examples map[string]json.RawMessage
	if err := json.Unmarshal(raw, &examples); err != nil {
		t.Fatal(err)
	}
	for path, expected := range examples {
		t.Run(path, func(t *testing.T) {
			fields, err := contract.ObjectFields(expected)
			if err != nil {
				t.Fatal(err)
			}
			child := &schemaObject{fields: fields}
			if branches := child.get("anyOf"); branches != nil {
				var values []json.RawMessage
				if err := json.Unmarshal(branches.(json.RawMessage), &values); err != nil {
					t.Fatal(err)
				}
				fields, err = contract.ObjectFields(values[0])
				if err != nil {
					t.Fatal(err)
				}
				child = &schemaObject{fields: fields}
			} else {
				var types []string
				if err := json.Unmarshal(child.get("type").(json.RawMessage), &types); err != nil {
					t.Fatal(err)
				}
				child.set("type", types[0])
			}
			got, err := contract.EncodeJSON(nullableSchema(child))
			if err != nil {
				t.Fatal(err)
			}
			var want bytes.Buffer
			if err := json.Compact(&want, expected); err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(got, want.Bytes()) {
				t.Fatalf("got %s, want %s", got, want.Bytes())
			}
		})
	}
}

// Ports the export-details portion of results_and_failures_print_the_documented_names.
func TestBrowserFailureExportDocument(t *testing.T) {
	raw, err := os.ReadFile("testdata/browser/failure.json")
	if err != nil {
		t.Fatal(err)
	}
	value, err := browser.DecodeBrowserFailure(bytes.TrimSpace(raw))
	if err != nil {
		t.Fatal(err)
	}
	if value.Details == nil || value.Details.AssetsExportResult == nil || len(value.Details.Files) != 1 || value.Details.Files[0].MIMEType != "image/png" {
		t.Fatalf("lost exported asset details: %+v", value)
	}
	got, err := contract.EncodeJSON(value)
	if err != nil || !bytes.Equal(got, bytes.TrimSpace(raw)) {
		t.Fatalf("failure document: %s %v", got, err)
	}
}

// These scalar spellings are from the existing independent Rust serde_json
// corpus in internal/contract/testdata/numbers.jsonl.
func TestBrowserScalarNumberBytes(t *testing.T) {
	for _, test := range []struct{ input, want string }{
		{"1.0", "1.0"},
		{"-0", "-0.0"},
		{"0.00001", "0.00001"},
		{"0.000001", "1e-6"},
		{"1000000000000000.0", "1000000000000000.0"},
		{"1e16", "1e+16"},
		{"5e-324", "5e-324"},
		{"1.7976931348623157e308", "1.7976931348623157e+308"},
	} {
		value, err := browser.DecodeNodeValue([]byte(test.input))
		if err != nil {
			t.Fatal(err)
		}
		got, err := contract.EncodeJSON(value)
		if err != nil || string(got) != test.want {
			t.Fatalf("%s: got %s %v, want %s", test.input, got, err, test.want)
		}
	}
}
