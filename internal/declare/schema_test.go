package declare_test

import (
	"encoding/json"
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/declare"
)

// Pure schema cases use no IO and finish within the ordinary one-second budget.
func TestSchemaChecksArgumentsWithoutDisclosingValues(t *testing.T) {
	schema, err := declare.NewSchema(json.RawMessage(`{"type":"object","additionalProperties":false,"properties":{"v":{"type":"array","items":{"type":"number"}},"quiet":{"type":"boolean"},"content":{"type":"string","maxLength":8},"count":{"type":"integer","minimum":1}},"required":["v"]}`))
	if err != nil {
		t.Fatal(err)
	}
	err = schema.Check(json.RawMessage(`{"v":["twelve"],"quiet":"maybe"}`))
	if err == nil {
		t.Fatal("invalid arguments accepted")
	}
	for _, want := range []string{`"v.0" is not of type "number"`, `"quiet" is not of type "boolean"`} {
		if !strings.Contains(err.Error(), want) {
			t.Errorf("%s lacks %s", err, want)
		}
	}
	for _, secret := range []string{"twelve", "maybe"} {
		if strings.Contains(err.Error(), secret) {
			t.Errorf("diagnostic discloses %s: %s", secret, err)
		}
	}
	for _, test := range []struct{ input, want string }{
		{`{"v":[1,1.5],"quiet":false}`, ""},
		{`{"v":[],"content":"a long body"}`, `"content" is longer than 8 characters`},
		{`{}`, `"v" is a required property`},
		{`{"v":[],"count":"7"}`, `"count" is not of type "integer"`},
		{`{"v":[],"extra":1}`, "Additional properties are not allowed ('extra' was unexpected)"},
	} {
		t.Run(test.input, func(t *testing.T) {
			err := schema.Check(json.RawMessage(test.input))
			if test.want == "" {
				if err != nil {
					t.Fatal(err)
				}
				return
			}
			if err == nil || err.Error() != test.want {
				t.Fatalf("got %v, want %s", err, test.want)
			}
		})
	}
}

func TestSchemaCompilationAndJSONBoundary(t *testing.T) {
	for _, document := range []string{`[]`, `null`, `{"type":"nonsense"}`, `{"type":"string","type":"number"}`, `{"$ref":"file:///nonexistent/demi-schema.json"}`, `{"$ref":"https://example.invalid/schema"}`} {
		t.Run(document, func(t *testing.T) {
			if _, err := declare.NewSchema(json.RawMessage(document)); err == nil {
				t.Fatal("invalid or external schema accepted")
			}
		})
	}
	raw := json.RawMessage(`{"type":"string","maxLength":1}`)
	schema, err := declare.NewSchema(raw)
	if err != nil {
		t.Fatal(err)
	}
	raw[0] = '['
	copied := schema.Document()
	copied[0] = '['
	if string(schema.Document()) != `{"type":"string","maxLength":1}` {
		t.Fatal("caller mutated schema")
	}
	if err := schema.Check(json.RawMessage(`"𝄞"`)); err != nil {
		t.Fatal(err)
	}
	if err := schema.Check(json.RawMessage(`"ab"`)); err == nil || err.Error() != "value is longer than 1 character" {
		t.Fatalf("got %v", err)
	}
	for _, document := range []string{`"a" "b"`, `"\ud800"`, `{"x":1,"x":2}`} {
		if err := schema.Check(json.RawMessage(document)); err == nil {
			t.Errorf("accepted corrupt JSON %s", document)
		}
	}
	var zero declare.Schema
	if err := zero.Check(json.RawMessage(`{}`)); err == nil {
		t.Fatal("zero schema accepted")
	}
}

func TestSchemaSupportsOutputAndLocalReferences(t *testing.T) {
	schema, err := declare.NewSchema(json.RawMessage(`{"$defs":{"item":{"type":"integer"}},"type":"object","properties":{"items":{"type":"array","items":{"$ref":"#/$defs/item"}}},"required":["items"]}`))
	if err != nil {
		t.Fatal(err)
	}
	if err := schema.Check(json.RawMessage(`{"items":[1,2]}`)); err != nil {
		t.Fatal(err)
	}
	if err := schema.Check(json.RawMessage(`{"items":[1,"private"]}`)); err == nil || err.Error() != `"items.1" is not of type "integer"` {
		t.Fatalf("got %v", err)
	}
}

// Port of the_input_subset_takes_scalars_enums_and_arrays_and_refuses_the_rest.
func TestInputSubset(t *testing.T) {
	accepted := `{"title":"Args","type":"object","additionalProperties":false,"required":["path"],"properties":{"path":{"type":"string","minLength":1,"description":"A file"},"count":{"type":"integer","format":"uint32","minimum":0},"mode":{"type":"string","enum":["fast","slow"]},"tags":{"type":"array","items":{"type":"string","pattern":"^[a-z]+$"},"maxItems":3},"force":{"type":"boolean"}}}`
	schema, err := declare.NewSchema(json.RawMessage(accepted))
	if err != nil {
		t.Fatal(err)
	}
	if err := declare.CheckInputSubset(schema); err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct{ field, reason string }{
		{`{"type":"integer","default":2}`, "carries a default"},
		{`{"type":["string","null"]}`, "allows null"},
		{`{"type":"string","enum":["a",null]}`, "allows null"},
		{`{"type":"object","properties":{"inner":{"type":"string"}}}`, "nested object"},
		{`{"oneOf":[{"type":"string"},{"type":"integer"}]}`, "union"},
		{`{"$ref":"#"}`, "refers to another schema"},
		{`{"type":"array","items":{"type":"array","items":{"type":"string"}}}`, "array of arrays"},
		{`{"type":"array","items":{"type":"string"},"uniqueItems":true}`, `"uniqueItems"`},
		{`{"type":"string","format":"uri"}`, "format"},
		{`true`, "must be a schema object"},
		{`{"type":"array"}`, "array without items"},
		{`{"type":"integer","enum":[1,2]}`, "not strings"},
		{`{"type":"string","enum":[1,2]}`, "not all strings"},
		{`{"type":["string","integer"]}`, "several types"},
		{`{}`, "declares no type"},
		{`{"type":"null"}`, `has type "null"`},
		{`{"type":"string","items":{"type":"string"}}`, "has items but is not an array"},
	} {
		t.Run(test.reason, func(t *testing.T) {
			document := fmt.Sprintf(`{"type":"object","additionalProperties":false,"properties":{"field":%s}}`, test.field)
			schema, err := declare.NewSchema(json.RawMessage(document))
			if err != nil {
				t.Fatal(err)
			}
			err = declare.CheckInputSubset(schema)
			if err == nil || !strings.HasPrefix(err.Error(), `input "field": `) || !strings.Contains(err.Error(), test.reason) {
				t.Fatalf("got %v, want %s", err, test.reason)
			}
		})
	}
	for _, test := range []struct{ document, reason string }{
		{`{"type":"object","properties":{}}`, "additionalProperties"},
		{`{"type":"array"}`, "must describe an object"},
		{`{"type":"object","additionalProperties":false,"required":["missing"]}`, "undeclared field"},
		{`{"type":"object","additionalProperties":false,"default":{}}`, `uses "default"`},
	} {
		schema, err := declare.NewSchema(json.RawMessage(test.document))
		if err != nil {
			t.Fatal(err)
		}
		if err := declare.CheckInputSubset(schema); err == nil || !strings.Contains(err.Error(), test.reason) {
			t.Errorf("got %v, want %s", err, test.reason)
		}
	}
}
