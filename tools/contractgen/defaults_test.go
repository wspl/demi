package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/tools/contractgen/testdata/external"
	"github.com/wspl/demi/tools/contractgen/testdata/presence"
)

// Default fields preserve Rust's absent/empty/value behavior in both formats.
// Local codecs and schema compilation only; budget below one second.
func TestDefaultFields(t *testing.T) {
	const empty = `{"items":[],"labels":{},"enabled":false,"count":0,"text":""}`
	const present = `{"items":["a"],"labels":{"a":"b"},"enabled":true,"count":2,"text":"yes"}`
	raw := presence.DefaultsJSONSchema()
	doc, err := jsonschema.UnmarshalJSON(bytes.NewReader(raw))
	if err != nil {
		t.Fatal(err)
	}
	compiler := jsonschema.NewCompiler()
	if err := compiler.AddResource("urn:defaults", doc); err != nil {
		t.Fatal(err)
	}
	schema, err := compiler.Compile("urn:defaults")
	if err != nil {
		t.Fatal(err)
	}
	for _, format := range []string{"JSON", "MessagePack"} {
		t.Run(format, func(t *testing.T) {
			decode := presence.DecodeDefaults
			encode := contract.EncodeJSON
			wire := func(input string) []byte { return []byte(input) }
			if format == "MessagePack" {
				decode = presence.DecodeDefaultsMsgpack
				encode = contract.EncodeMsgpack
				wire = func(input string) []byte {
					data, err := contract.EncodeMsgpack(json.RawMessage(input))
					if err != nil {
						t.Fatal(err)
					}
					return data
				}
			}
			for _, input := range []string{`{}`, empty, present} {
				value, err := decode(wire(input))
				if err != nil {
					t.Fatalf("%s: %v", input, err)
				}
				if value.Items == nil || value.Labels == nil {
					t.Fatal("default collections decoded as nil")
				}
				data, err := encode(value)
				if err != nil {
					t.Fatal(err)
				}
				want := input
				if input == `{}` {
					want = empty
				}
				if !bytes.Equal(data, wire(want)) {
					t.Fatalf("encoded %x; want %x", data, wire(want))
				}
				document, err := jsonschema.UnmarshalJSON(bytes.NewReader([]byte(input)))
				if err != nil {
					t.Fatal(err)
				}
				if err := schema.Validate(document); err != nil {
					t.Fatalf("schema rejects %s: %v", input, err)
				}
			}
			for _, input := range []string{`{"items":null}`, `{"labels":null}`, `{"enabled":null}`, `{"count":null}`, `{"text":null}`, `{"count":11}`, `{"text":"longer"}`, `{"items":["a","b","c"]}`} {
				if _, err := decode(wire(input)); err == nil {
					t.Errorf("accepted %s", input)
				}
				document, err := jsonschema.UnmarshalJSON(bytes.NewReader([]byte(input)))
				if err != nil {
					t.Fatal(err)
				}
				if err := schema.Validate(document); err == nil {
					t.Errorf("schema accepted %s", input)
				}
			}
			zero := presence.Defaults{}
			if err := zero.Validate(); err != nil {
				t.Fatal(err)
			}
			data, err := encode(zero)
			if err != nil || !bytes.Equal(data, wire(empty)) {
				t.Fatalf("zero encoding: %x, %v", data, err)
			}
			if zero.Items != nil || zero.Labels != nil {
				t.Fatal("encoding mutated caller")
			}
			bad := presence.Defaults{Count: 11}
			if err := bad.Validate(); err == nil {
				t.Fatal("default bypassed validation")
			}
			if _, err := encode(bad); err == nil {
				t.Fatal("encoded invalid default field")
			}
		})
	}
	// Schemars emits Default::default() as metadata, without making null legal.
	want := `{"additionalProperties":false,"properties":{"items":{"type":"array","items":{"type":"string"},"maxItems":2,"default":[]},"labels":{"type":"object","additionalProperties":{"type":"string"},"default":{}},"enabled":{"type":"boolean","default":false},"count":{"type":"integer","format":"uint32","minimum":0,"maximum":10,"default":0},"text":{"type":"string","maxLength":4,"default":""}},"title":"Defaults","type":"object"}`
	if string(raw) != want {
		t.Fatalf("schema = %s; want %s", raw, want)
	}
}

// A consumer uses the owning package's normalizer and error without an adapter.
// This two-package fixture loads local declarations only; budget two seconds.
func TestImportedExposeCodec(t *testing.T) {
	if err := generate(t.Context(), []string{"./testdata/external"}, false, "", true); err != nil {
		t.Fatal(err)
	}
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/external"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	source, err := os.ReadFile(filepath.Join(dest, "plugin-external", "plugin.ts"))
	if err != nil {
		t.Fatal(err)
	}
	// Rust plugin-expose exports this exact plain string form. Validation and
	// bare-port normalization belong to the Go codec, not the page schema.
	if !strings.Contains(string(source), "const exposeAddressSchema = z.string()\n") {
		t.Fatalf("expose address mapping: %s", source)
	}
	// Captured from the Rust expose page's $defs.ExposeAddress.
	expected, err := os.ReadFile("testdata/external/expose-address.json")
	if err != nil {
		t.Fatal(err)
	}
	var schema struct {
		Properties map[string]json.RawMessage `json:"properties"`
	}
	if err := json.Unmarshal(external.ExposeJSONSchema(), &schema); err != nil {
		t.Fatal(err)
	}
	var compact bytes.Buffer
	if err := json.Compact(&compact, expected); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(schema.Properties["address"], compact.Bytes()) {
		t.Fatalf("expose address schema: %s; Rust: %s", schema.Properties["address"], compact.Bytes())
	}

	value, err := external.DecodeExpose([]byte(`{"address":"08080"}`))
	if err != nil {
		t.Fatal(err)
	}
	if value.Address != "127.0.0.1:8080" {
		t.Fatalf("address = %q", value.Address)
	}
	data, err := contract.EncodeJSON(external.Expose{Address: "08080"})
	if err != nil || string(data) != `{"address":"127.0.0.1:8080"}` {
		t.Fatalf("encode: %s, %v", data, err)
	}
	for _, input := range []string{`{}`, `{"address":null}`, `{"address":5}`} {
		if _, err := external.DecodeExpose([]byte(input)); err == nil {
			t.Errorf("accepted %s", input)
		}
	}
	invalid := external.Expose{Address: "0"}
	_, wantError := invalid.Address.Port()
	_, err = external.DecodeExpose([]byte(`{"address":"0"}`))
	if !errors.Is(err, wantError) {
		t.Fatalf("lost codec error: %v", err)
	}
	_, err = contract.EncodeJSON(external.Expose{Address: "0"})
	if !errors.Is(err, wantError) {
		t.Fatalf("lost encoder error: %v", err)
	}
}
