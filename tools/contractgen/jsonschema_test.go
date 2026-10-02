package main

import (
	"bytes"
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/wspl/demi/tools/contractgen/testdata/schemas"
)

// Each scenario compiles a draft 2020-12 schema and checks the same inputs at
// both command boundaries. Budget: one second, no processes or network.
func TestCommandSchemas(t *testing.T) {
	cases := []struct {
		name    string
		schema  func() json.RawMessage
		decode  func([]byte) error
		valid   []string
		invalid []string
	}{
		{"ReadArgs", schemas.ReadArgsJSONSchema, decodeSchemaValue(schemas.DecodeReadArgs), []string{`{"path":"a"}`}, []string{`{}`, `{"path":null}`, `{"path":"a","extra":true}`}},
		{"CreateArgs", schemas.CreateArgsJSONSchema, decodeSchemaValue(schemas.DecodeCreateArgs), []string{`{"path":"a","content":""}`}, []string{`{"path":"a"}`, `{"path":"a","content":4}`}},
		{"EditArgs", schemas.EditArgsJSONSchema, decodeSchemaValue(schemas.DecodeEditArgs), []string{`{"path":"a","old":"x","new":"y","occurrence":2}`, `{"path":"a","old":"😀","new":""}`}, []string{`{"path":"a","old":"","new":"y"}`, `{"path":"a","old":"x","new":"y","occurrence":0}`, `{"path":"a","old":"x","new":"y","context":0}`, `{"path":"a","old":"x","new":"y","context":null}`, `{"path":"a","old":"x"}`, `{"path":"a","old":"x","new":"y","occurrence":18446744073709551616}`}},
		{"PatchArgs", schemas.PatchArgsJSONSchema, decodeSchemaValue(schemas.DecodePatchArgs), []string{`{"patch":""}`}, []string{`{}`, `{"patch":[]}`}},
		{"OpenInput", schemas.OpenInputJSONSchema, decodeSchemaValue(schemas.DecodeOpenInput), []string{`{"url":"https://example.test","load":"commit","timeout":300000}`, `{"url":"x"}`}, []string{`{"url":""}`, `{"url":"x","timeout":0}`, `{"url":"x","timeout":300001}`, `{"url":"x","load":"other"}`, `{"url":"x","load":null}`}},
		{"NavigationResult", schemas.NavigationResultJSONSchema, decodeSchemaValue(schemas.DecodeNavigationResult), []string{`{"tab":"t1","url":"x"}`, `{"tab":"t9","url":"x","title":""}`}, []string{`{"tab":"t0","url":"x"}`, `{"tab":"t1","url":"x","title":null}`, `{"tab":"t1"}`}},
		{"CloseResult", schemas.CloseResultJSONSchema, decodeSchemaValue(schemas.DecodeCloseResult), []string{`{"closed":"t12"}`}, []string{`{"closed":"x12"}`, `{}`}},
		{"ExampleArgs", schemas.ExampleArgsJSONSchema, decodeSchemaValue(schemas.DecodeExampleArgs), []string{`{"path":"a","tags":[],"mode":"fast","count":9}`, `{"path":"a","tags":["x"],"labels":[],"no-cache":false}`}, []string{`{"path":"a"}`, `{"path":"a","tags":[],"count":10}`, `{"path":"a","tags":[],"labels":null}`, `{"path":"a","tags":[],"mode":"other"}`}},
		{"Outcome", schemas.OutcomeJSONSchema, decodeSchemaValue(schemas.DecodeOutcome), []string{`{"kind":"ok","value":null}`, `{"kind":"error","message":"x"}`}, []string{`{"kind":"ok"}`, `{"kind":"other"}`, `{"kind":"ok","value":3}`}},
		{"Constraints", schemas.ConstraintsJSONSchema, decodeSchemaValue(schemas.DecodeConstraints), []string{`{"number":2,"choice":"fast","text":"ab"}`}, []string{`{"number":1,"choice":"fast","text":"ab"}`, `{"number":8,"choice":"fast","text":"ab"}`, `{"number":2,"choice":"other","text":"ab"}`, `{"number":2,"choice":"slow","text":"ab"}`, `{"number":2,"choice":"fast","text":"a"}`, `{"number":2,"choice":"fast","text":"abcd"}`, `{"number":2,"choice":"fast","text":"bb"}`}},
		{"Collection", schemas.CollectionJSONSchema, decodeSchemaValue(schemas.DecodeCollection), []string{`{"values":[1,8],"labels":{"a":null},"extra":true}`}, []string{`{"values":[],"labels":{}}`, `{"values":[9],"labels":{}}`, `{"values":[1,2,3],"labels":{}}`, `{"values":[1],"labels":{"a":3}}`}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			raw := tc.schema()
			// Each caller owns its bytes; mutating one result cannot poison a manifest.
			raw[0] = '!'
			raw = tc.schema()
			doc, err := jsonschema.UnmarshalJSON(bytes.NewReader(raw))
			if err != nil {
				t.Fatal(err)
			}
			expected, err := os.ReadFile("testdata/schemas/" + tc.name + ".json")
			if err != nil {
				t.Fatal(err)
			}
			want, err := jsonschema.UnmarshalJSON(bytes.NewReader(expected))
			if err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(doc, want) {
				t.Fatalf("schema differs from expectation:\ngot %s\nwant %s", raw, expected)
			}
			c := jsonschema.NewCompiler()
			c.DefaultDraft(jsonschema.Draft2020)
			if err := c.AddResource("https://fixtures.test/schema", doc); err != nil {
				t.Fatal(err)
			}
			compiled, err := c.Compile("https://fixtures.test/schema")
			if err != nil {
				t.Fatal(err)
			}
			for _, group := range []struct {
				inputs   []string
				accepted bool
			}{{tc.valid, true}, {tc.invalid, false}} {
				for _, input := range group.inputs {
					value, err := jsonschema.UnmarshalJSON(strings.NewReader(input))
					if err != nil {
						t.Fatal(err)
					}
					schemaErr := compiled.Validate(value)
					decodeErr := tc.decode([]byte(input))
					if (schemaErr == nil) != group.accepted || (decodeErr == nil) != group.accepted {
						t.Errorf("%s: want accepted=%v; schema=%v decoder=%v", input, group.accepted, schemaErr, decodeErr)
					}
				}
			}
		})
	}
}

// decodeSchemaValue adapts generated command decoders to the shared scenarios.
func decodeSchemaValue[T any](decode func([]byte) (T, error)) func([]byte) error {
	return func(data []byte) error {
		_, err := decode(data)
		return err
	}
}
