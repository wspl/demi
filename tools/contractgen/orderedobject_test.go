package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"errors"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/tools/contractgen/testdata/orderedobject"
)

// Object fields preserve member order and use the reference encoding through
// containing contracts and arrays. Local fixture only; budget <1 second.
func TestOrderedObjectRoundTrip(t *testing.T) {
	data, err := os.ReadFile("testdata/orderedobject/encodings.jsonl")
	if err != nil {
		t.Fatal(err)
	}
	scanner := bufio.NewScanner(bytes.NewReader(data))
	for scanner.Scan() {
		var row struct{ Input, Object, Contract string }
		if err := json.Unmarshal(scanner.Bytes(), &row); err != nil {
			t.Fatal(err)
		}
		raw := row.Input
		wire := `{"data":` + raw + `}`
		value, err := orderedobject.DecodeInput([]byte(wire))
		if err != nil {
			t.Fatal(err)
		}
		if string(value.Data) != raw {
			t.Fatalf("data got %s, want %s", value.Data, raw)
		}
		encoded, err := value.MarshalJSON()
		if err != nil || string(encoded) != row.Contract {
			t.Fatalf("MarshalJSON got %s, %v; want %s", encoded, err, row.Contract)
		}
		fields, err := contract.ObjectFields(value.Data)
		if err != nil {
			t.Fatal(err)
		}
		if len(fields) > 0 && (fields[0].Name != "z" || fields[1].Name != "a") {
			t.Fatalf("fields got %v, want z then a", fields)
		}
		for _, tc := range []struct {
			value any
			want  string
		}{
			{value.Data, row.Object},
			{oracleJSON(" \n" + strings.ReplaceAll(row.Object, "<", `\u003c`) + "\n "), row.Object},
			{value, row.Contract},
			{orderedobject.Envelope{Input: value}, `{"input":` + row.Contract + `}`},
			{[]orderedobject.Input{value}, `[` + row.Contract + `]`},
		} {
			got, err := contract.EncodeJSON(tc.value)
			if err != nil || string(got) != tc.want {
				t.Fatalf("encoding got %s, %v; want %s", got, err, tc.want)
			}
		}
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
}

// oracleJSON exercises an opaque codec that returns the original JSON spelling.
type oracleJSON json.RawMessage

func (v oracleJSON) MarshalJSON() ([]byte, error) { return []byte(v), nil }

// The fixture records the object schema and non-object errors from the reference
// decoder. Local fixture only; budget <1 second.
func TestOrderedObjectRefusalsAndSchema(t *testing.T) {
	data, err := os.ReadFile("testdata/orderedobject/refusals.jsonl")
	if err != nil {
		t.Fatal(err)
	}
	scanner := bufio.NewScanner(bytes.NewReader(data))
	if !scanner.Scan() {
		t.Fatal("missing schema fixture")
	}
	var want any
	if err := json.Unmarshal(scanner.Bytes(), &want); err != nil {
		t.Fatal(err)
	}
	for _, schema := range []json.RawMessage{orderedobject.InputJSONSchema(), orderedobject.InputPluginJSONSchema()} {
		var got any
		if err := json.Unmarshal(schema, &got); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("schema got %v, want %v", got, want)
		}
	}
	for scanner.Scan() {
		var row struct{ Input, Error string }
		if err := json.Unmarshal(scanner.Bytes(), &row); err != nil {
			t.Fatal(err)
		}
		_, err := orderedobject.DecodeInput([]byte(`{"data":` + row.Input + `}`))
		var refusal *contract.Error
		message, _, _ := strings.Cut(row.Error, " at line ")
		if !errors.As(err, &refusal) || refusal.Path != "data" || refusal.Err.Error() != message {
			t.Fatalf("decode %s got %v, want data: %s", row.Input, err, message)
		}
		value := orderedobject.Input{Data: json.RawMessage(row.Input)}
		if _, err := contract.EncodeJSON(value); err == nil {
			t.Fatalf("encoder accepted %s", row.Input)
		}
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
	for _, raw := range []string{`{"data":{"x":}}`, `{"data":{"x":1,"x":2}}`, `{}`} {
		if _, err := orderedobject.DecodeInput([]byte(raw)); err == nil {
			t.Fatalf("decoder accepted %s", raw)
		}
	}
}

// Generated Zod and its inferred TypeScript type share the object restriction.
// One local package load; budget 2 seconds, no network.
func TestOrderedObjectTypeScript(t *testing.T) {
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/orderedobject"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	got, err := os.ReadFile(filepath.Join(dest, "protocol", "contracts.ts"))
	if err != nil {
		t.Fatal(err)
	}
	want, err := os.ReadFile("testdata/orderedobject/contracts.ts")
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Fatalf("TypeScript got %s, want %s", got, want)
	}
}

// Generated scalar and collection codecs retain float32 spelling even when a
// surrounding encoder normalizes raw JSON. Local oracle fixture; budget <1 second.
func TestOrderedObjectNumberEncoding(t *testing.T) {
	data, err := os.ReadFile("testdata/orderedobject/numbers.json")
	if err != nil {
		t.Fatal(err)
	}
	var row struct{ Scalar, List, Contract string }
	if err := json.Unmarshal(data, &row); err != nil {
		t.Fatal(err)
	}
	values := []orderedobject.Float32{1e13, 1.5, orderedobject.Float32(math.Copysign(0, -1))}
	for _, tc := range []struct {
		value any
		want  string
	}{
		{values[0], row.Scalar},
		{values, row.List},
		{orderedobject.FloatValues{Values: values}, row.Contract},
	} {
		got, err := contract.EncodeJSON(tc.value)
		if err != nil || string(got) != tc.want {
			t.Fatalf("encoding got %s, %v; want %s", got, err, tc.want)
		}
	}
}
