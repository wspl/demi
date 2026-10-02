package main

import (
	"encoding/json"
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/tools/contractgen/testdata/presence"
	"github.com/wspl/demi/tools/contractgen/testdata/schemacheck"
)

// Schema roots retain structural rules but omit custom Go rules, including
// checks on flattened children. Local generated calls; budget below one second.
func TestSchemaGoOnlyChecks(t *testing.T) {
	for _, tc := range []struct {
		name           string
		schema         json.RawMessage
		want           string
		decode         func([]byte) error
		good, rejected string
	}{
		{"scalar", schemacheck.CheckedTextJSONSchema(), `{"title":"CheckedText","type":"string","minLength":1}`, decodeSchemaValue(schemacheck.DecodeCheckedText), `"allowed"`, `"reserved"`},
		{"embedded", schemacheck.EnvelopeJSONSchema(), `{"title":"Envelope","type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}`, decodeSchemaValue(schemacheck.DecodeEnvelope), `{"value":"allowed"}`, `{"value":"reserved"}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var got, want any
			if err := json.Unmarshal(tc.schema, &got); err != nil {
				t.Fatal(err)
			}
			if err := json.Unmarshal([]byte(tc.want), &want); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(got, want) {
				t.Fatalf("schema includes more than structural rules: %s", tc.schema)
			}
			if err := tc.decode([]byte(tc.good)); err != nil {
				t.Fatal(err)
			}
			if err := tc.decode([]byte(tc.rejected)); !errors.Is(err, schemacheck.ErrReserved) {
				t.Fatalf("custom check did not reject input: %v", err)
			}
		})
	}
	if _, err := schemacheck.DecodeCheckedText([]byte(`""`)); err == nil {
		t.Fatal("structural length rule was lost")
	}
}

// Schemars omits a skipped None default, makes Option fields optional, and adds
// null to their type. Exact schema comparison; local CPU budget <1 second.
func TestNullableOptionalSchema(t *testing.T) {
	want := `{"title":"Patch","type":"object","additionalProperties":false,"properties":{"option":{"type":["string","null"],"maxLength":4},"double":{"type":["string","null"],"maxLength":4},"items":{"type":["array","null"],"items":{"type":"string"}}}}`
	var actual, expected any
	if err := json.Unmarshal(presence.PatchJSONSchema(), &actual); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal([]byte(want), &expected); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(actual, expected) {
		t.Fatalf("optional-null schema: %s; want %s", presence.PatchJSONSchema(), want)
	}
}

// The schema's character bounds complement the owner's unchanged byte checks.
// This reaches the actual runnerwire type through an outside schema root; <1 s.
func TestInstallSchemaBounds(t *testing.T) {
	var schema struct {
		Properties map[string]struct {
			Properties map[string]struct {
				MinLength int `json:"minLength"`
				MaxLength int `json:"maxLength"`
			} `json:"properties"`
		} `json:"properties"`
	}
	if err := json.Unmarshal(presence.InstallEnvelopeJSONSchema(), &schema); err != nil {
		t.Fatal(err)
	}
	for name, maximum := range map[string]int{"package": 200, "name": 100, "version": 100} {
		field := schema.Properties["install"].Properties[name]
		if field.MinLength != 1 || field.MaxLength != maximum {
			t.Errorf("%s bounds: %+v; want 1..%d", name, field, maximum)
		}
	}
}
