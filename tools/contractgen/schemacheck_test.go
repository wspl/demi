package main

import (
	"encoding/json"
	"errors"
	"reflect"
	"testing"

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
