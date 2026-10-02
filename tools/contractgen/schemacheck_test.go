package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
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

// Schema-only bounds preserve schema bytes while the owner controls Go errors.
// In-memory generated boundary calls; budget below one second.
func TestSchemaOnlyRange(t *testing.T) {
	plain := strings.Replace(string(schemacheck.PlainRangeJSONSchema()), "PlainRange", "SchemaRange", 1)
	if got := string(schemacheck.SchemaRangeJSONSchema()); got != plain {
		t.Fatalf("schema = %s; want %s", got, plain)
	}
	for _, raw := range []string{`{"value":0}`, `{"value":11}`} {
		if _, err := schemacheck.DecodeSchemaRange([]byte(raw)); err != nil {
			t.Fatal(err)
		}
		if _, err := schemacheck.DecodePlainRange([]byte(raw)); err == nil {
			t.Fatal("plain range accepted out-of-range input")
		}
	}
	for _, tc := range []struct{ raw, want string }{
		{`{"number":0,"stdin":"x"}`, "number: reserved value"},
		{`{"number":11,"stdin":"x"}`, "number: reserved value"},
		{`{"number":4,"stdin":"x"}`, "number: reserved value"},
		{`{"number":5,"stdin":""}`, "stdin: must not be empty; use shell_status to poll"},
		{`{"number":5,"stdin":"x"}`, ""},
	} {
		_, err := schemacheck.DecodeCheckedInput([]byte(tc.raw))
		if tc.want == "" {
			if err != nil {
				t.Fatal(err)
			}
		} else if err == nil || err.Error() != tc.want {
			t.Fatalf("%s: got %v; want %s", tc.raw, err, tc.want)
		}
	}
}

// Zod must retain the same bounds for schema-only and ordinary ranges.
// One fixture package load, no JavaScript process; budget below one second.
func TestSchemaOnlyRangeZod(t *testing.T) {
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/schemacheck"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(dest, "plugin-schemacheck", "plugin.ts"))
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"plainRange", "schemaRange"} {
		want := "export const " + name + "Schema = z.object({\"value\": z.number().min(1).max(10)})"
		if !strings.Contains(string(data), want) {
			t.Fatalf("missing %s in %s", want, data)
		}
	}
}
