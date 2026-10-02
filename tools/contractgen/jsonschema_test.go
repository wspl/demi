package main

import (
	"bytes"
	"encoding/json"
	"go/parser"
	"go/token"
	"os"
	"reflect"
	"slices"
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
		{"EditArgs", schemas.EditArgsJSONSchema, decodeSchemaValue(schemas.DecodeEditArgs), []string{`{"path":"a","old":"x","new":"y","occurrence":2}`, `{"path":"a","old":"😀","new":""}`}, []string{`{"path":"a","old":"","new":"y"}`, `{"path":"a","old":"x","new":"y","occurrence":0}`, `{"path":"a","old":"x","new":"y","context":0}`, `{"path":"a","old":"x","new":"y","context":null}`, `{"path":"a","old":"x"}`}},
		{"PatchArgs", schemas.PatchArgsJSONSchema, decodeSchemaValue(schemas.DecodePatchArgs), []string{`{"patch":""}`}, []string{`{}`, `{"patch":[]}`}},
		{"OpenInput", schemas.OpenInputJSONSchema, decodeSchemaValue(schemas.DecodeOpenInput), []string{`{"url":"https://example.test","load":"commit","timeout":300000}`, `{"url":"x"}`}, []string{`{"url":""}`, `{"url":"x","timeout":0}`, `{"url":"x","timeout":300001}`, `{"url":"x","load":"other"}`, `{"url":"x","load":null}`}},
		{"NavigationResult", schemas.NavigationResultJSONSchema, decodeSchemaValue(schemas.DecodeNavigationResult), []string{`{"tab":"t1","url":"x"}`, `{"tab":"t9","url":"x","title":""}`}, []string{`{"tab":"t0","url":"x"}`, `{"tab":"t1","url":"x","title":null}`, `{"tab":"t1"}`}},
		{"CloseResult", schemas.CloseResultJSONSchema, decodeSchemaValue(schemas.DecodeCloseResult), []string{`{"closed":"t12"}`}, []string{`{"closed":"x12"}`, `{}`}},
		{"ExampleArgs", schemas.ExampleArgsJSONSchema, decodeSchemaValue(schemas.DecodeExampleArgs), []string{`{"path":"a","tags":[],"mode":"fast","count":9}`, `{"path":"a","tags":["x"],"labels":[],"no-cache":false}`}, []string{`{"path":"a"}`, `{"path":"a","tags":[],"count":10}`, `{"path":"a","tags":[],"labels":null}`, `{"path":"a","tags":[],"mode":"other"}`}},
		{"OpenResult", schemas.OpenResultJSONSchema, decodeSchemaValue(schemas.DecodeOpenResult), []string{`{"tab":"t1","url":"x","viewport":{"width":1,"height":1,"devicePixelRatio":1,"mode":"web"}}`}, []string{`{"tab":"t1","url":"x","viewport":{"width":0,"height":1,"devicePixelRatio":1,"mode":"web"}}`}},
		{"GotoInput", schemas.GotoInputJSONSchema, decodeSchemaValue(schemas.DecodeGotoInput), []string{`{"tab":"t1","url":"x"}`}, []string{`{"tab":"t1","url":""}`}},
		{"BackInput", schemas.BackInputJSONSchema, decodeSchemaValue(schemas.DecodeBackInput), []string{`{"tab":"t1"}`}, []string{`{"tab":"t1","load":null}`}},
		{"ForwardInput", schemas.ForwardInputJSONSchema, decodeSchemaValue(schemas.DecodeForwardInput), []string{`{"tab":"t1"}`}, []string{`{"tab":"t1","timeout":0}`}},
		{"ReloadInput", schemas.ReloadInputJSONSchema, decodeSchemaValue(schemas.DecodeReloadInput), []string{`{"tab":"t1"}`}, []string{`{"tab":"t0"}`}},
		{"CloseInput", schemas.CloseInputJSONSchema, decodeSchemaValue(schemas.DecodeCloseInput), []string{`{"tab":"t1"}`}, []string{`{"tab":"t1","timeout":null}`}},
		{"ViewportSetInput", schemas.ViewportSetInputJSONSchema, decodeSchemaValue(schemas.DecodeViewportSetInput), []string{`{"tab":"t1","width":4096,"height":1,"scale":4}`}, []string{`{"tab":"t1","width":4097,"height":1}`, `{"tab":"t1","width":1,"height":1,"scale":0.4}`}},
		{"ViewportResetInput", schemas.ViewportResetInputJSONSchema, decodeSchemaValue(schemas.DecodeViewportResetInput), []string{`{"tab":"t1"}`}, []string{`{"tab":"t1","width":1}`}},
		{"ViewportResult", schemas.ViewportResultJSONSchema, decodeSchemaValue(schemas.DecodeViewportResult), []string{`{"viewport":{"width":1,"height":1,"devicePixelRatio":1,"mode":"mobile"}}`}, []string{`{"viewport":{"width":1,"height":1,"devicePixelRatio":0,"mode":"web"}}`}},
		{"CdpDetachInput", schemas.CdpDetachInputJSONSchema, decodeSchemaValue(schemas.DecodeCdpDetachInput), []string{`{"tab":"t1"}`}, []string{`{"tab":"t1","timeout":300001}`}},
		{"CdpDetachResult", schemas.CdpDetachResultJSONSchema, decodeSchemaValue(schemas.DecodeCdpDetachResult), []string{`{"detached":"t1"}`}, []string{`{"detached":"t0"}`}},
		{"WebmcpCallInput", schemas.WebmcpCallInputJSONSchema, decodeSchemaValue(schemas.DecodeWebmcpCallInput), []string{`{"tab":"t1","tool":"x","tools":"y","arguments":"{}"}`}, []string{`{"tab":"t1","tool":"","tools":"y","arguments":"{}"}`}},
		{"WebmcpCallResult", schemas.WebmcpCallResultJSONSchema, decodeSchemaValue(schemas.DecodeWebmcpCallResult), []string{`{"name":"x","result":null}`, `{"name":"x","result":{"a":[1,true]}}`}, []string{`{"name":"x"}`}},
		{"ExposeAnswer", schemas.ExposeAnswerJSONSchema, decodeSchemaValue(schemas.DecodeExposeAnswer), []string{`{"expose":{"number":1,"device":"x","address":"x","url":"x","expiresAt":"2026-01-01T00:00:00.000Z"}}`}, []string{`{"expose":{"number":1,"device":"x","address":"x","url":"x","expiresAt":"invalid"}}`}},
		{"ExposeLines", schemas.ExposeLinesJSONSchema, decodeSchemaValue(schemas.DecodeExposeLines), []string{`{"exposes":[]}`}, []string{`{"exposes":null}`}},
		{"Numeric", schemas.NumericJSONSchema, decodeSchemaValue(schemas.DecodeNumeric), []string{`{"rune":65,"byte":0,"i8":-128,"i16":32767,"i32":1,"i64":1,"int":1,"u8":255,"u16":65535,"u32":1,"u64":1,"uint":1,"f32":0.5,"f64":2}`}, []string{`{"rune":65,"byte":0,"i8":-129,"i16":0,"i32":0,"i64":0,"int":0,"u8":0,"u16":0,"u32":0,"u64":0,"uint":0,"f32":0,"f64":0}`}},
		{"Recursive", schemas.RecursiveJSONSchema, decodeSchemaValue(schemas.DecodeRecursive), []string{`{"next":{"next":{}}}`}, []string{`{"next":null}`, `{"next":{"bad":true}}`}},
		{"RecursiveResult", schemas.RecursiveResultJSONSchema, decodeSchemaValue(schemas.DecodeRecursiveResult), []string{`{"tree":{"next":{}}}`}, []string{`{"tree":{"next":null}}`}},
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
			// Real command schemas are compared directly with the captured
			// manifest in TestRustManifestSchemas; these are generator-only shapes.
			if slices.Contains([]string{"ExampleArgs", "Outcome", "Collection", "Constraints", "Numeric", "Recursive", "RecursiveResult"}, tc.name) {
				expected, err := os.ReadFile("testdata/schemas/" + tc.name + ".json")
				if err != nil {
					t.Fatal(err)
				}
				want, err := jsonschema.UnmarshalJSON(bytes.NewReader(expected))
				if err != nil {
					t.Fatal(err)
				}
				if !reflect.DeepEqual(doc, want) {
					t.Fatalf("schema differs:\ngot %s\nwant %s", raw, expected)
				}
			}
			c := jsonschema.NewCompiler()
			c.DefaultDraft(jsonschema.Draft2020)
			c.AssertFormat()
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

// TestRustManifestSchemas pins product annotations as well as validation. It
// compares whole canonical JSON values, retaining array order (including required).
// Local fixture processing costs less than one second and uses no network.
func TestRustManifestSchemas(t *testing.T) {
	generated := map[string]func() json.RawMessage{
		"ReadArgs": schemas.ReadArgsJSONSchema, "CreateArgs": schemas.CreateArgsJSONSchema,
		"EditArgs": schemas.EditArgsJSONSchema, "PatchArgs": schemas.PatchArgsJSONSchema,
		"OpenInput": schemas.OpenInputJSONSchema, "OpenResult": schemas.OpenResultJSONSchema,
		"GotoInput": schemas.GotoInputJSONSchema, "BackInput": schemas.BackInputJSONSchema,
		"ForwardInput": schemas.ForwardInputJSONSchema, "ReloadInput": schemas.ReloadInputJSONSchema,
		"NavigationResult": schemas.NavigationResultJSONSchema, "CloseInput": schemas.CloseInputJSONSchema,
		"CloseResult": schemas.CloseResultJSONSchema, "ViewportSetInput": schemas.ViewportSetInputJSONSchema,
		"ViewportResetInput": schemas.ViewportResetInputJSONSchema, "ViewportResult": schemas.ViewportResultJSONSchema,
		"CdpDetachInput": schemas.CdpDetachInputJSONSchema, "CdpDetachResult": schemas.CdpDetachResultJSONSchema,
		"WebmcpCallInput": schemas.WebmcpCallInputJSONSchema, "WebmcpCallResult": schemas.WebmcpCallResultJSONSchema,
		"ExposeAnswer": schemas.ExposeAnswerJSONSchema, "ExposeLines": schemas.ExposeLinesJSONSchema,
	}
	raw, err := os.ReadFile("testdata/schemas/manifests.json")
	if err != nil {
		t.Fatal(err)
	}
	var manifests []map[string]any
	if err := json.Unmarshal(raw, &manifests); err != nil {
		t.Fatal(err)
	}
	counts := map[string]int{}
	var visit func(string, map[string]any)
	visit = func(plugin string, node map[string]any) {
		if children, ok := node["subcommands"].([]any); ok {
			for _, child := range children {
				visit(plugin, child.(map[string]any))
			}
			return
		}
		input, _ := node["input"].(map[string]any)
		output, _ := node["output"].(map[string]any)
		result, _ := output["json"].(map[string]any)
		inputTitle, _ := input["title"].(string)
		if plugin == "browser" && generated[inputTitle] == nil {
			return
		}
		if plugin != "browser" && plugin != "file" && plugin != "expose" {
			return
		}
		if plugin == "expose" && result == nil {
			return
		}
		counts[plugin]++
		for _, side := range []struct {
			name  string
			value map[string]any
		}{{"input", input}, {"output", result}} {
			if side.value == nil || plugin == "expose" && side.name == "input" {
				continue
			}
			title := side.value["title"].(string)
			emit := generated[title]
			if emit == nil {
				t.Fatalf("missing Go schema for %s/%s", plugin, title)
			}
			t.Run(plugin+"/"+node["name"].(string)+"/"+side.name, func(t *testing.T) {
				var got map[string]any
				if err := json.Unmarshal(emit(), &got); err != nil {
					t.Fatal(err)
				}
				if plugin == "browser" && side.name == "input" {
					// plugin-browser::leaf, through LeafBuilder::describe, replaces only
					// this description. The original type doc must remain in generated output.
					properties := got["properties"].(map[string]any)
					timeout := properties["timeout"].(map[string]any)
					if timeout["description"] != "Whole operation deadline in milliseconds" {
						t.Fatalf("lost timeout type documentation: %v", timeout)
					}
					deadline := "30000"
					if title == "OpenInput" {
						deadline = "300000"
					}
					timeout["description"] = "Whole operation deadline in milliseconds; default " + deadline + ", maximum 300000."
				}
				actual, err := json.Marshal(got)
				if err != nil {
					t.Fatal(err)
				}
				expected, err := json.Marshal(side.value)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(actual, expected) {
					t.Fatalf("Rust manifest differs:\nGo:   %s\nRust: %s", actual, expected)
				}
			})
		}
	}
	for _, plugin := range manifests {
		for _, command := range plugin["commands"].([]any) {
			visit(plugin["id"].(string), command.(map[string]any)["tree"].(map[string]any))
		}
	}
	if counts["file"] != 4 || counts["browser"] != 10 || counts["expose"] != 3 {
		t.Fatalf("leaf coverage changed: %v", counts)
	}
}

// The Go formatter may normalize source comments, but generation must not
// independently rewrite the product text it receives. Budget: under 1 ms.
func TestContractDescriptionPreservesWhitespace(t *testing.T) {
	source := "package fixture\n// First line.\n//\n//\n// Second paragraph.\n//     indented example\n// +demi:schema\ntype Input string\n"
	file, err := parser.ParseFile(token.NewFileSet(), "types.go", source, parser.ParseComments)
	if err != nil {
		t.Fatal(err)
	}
	want := "First line.\n\n\nSecond paragraph.\n    indented example"
	if got := contractDescription(file.Comments[0]); got != want {
		t.Fatalf("description changed: got %q, want %q", got, want)
	}
}
