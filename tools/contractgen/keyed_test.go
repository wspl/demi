package main

import (
	"bytes"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/tools/contractgen/testdata/keyed"
	"github.com/wspl/demi/tools/contractgen/testdata/runner"
	"github.com/wspl/demi/tools/contractgen/testdata/unions"
)

// Frame callers must distinguish malformed text from a well-formed invalid value.
// Includes the Rust a_message_that_is_not_json_is_told_from_an_invalid_frame cases.
// In-memory boundary tests cost under one second and own no resources.
func TestSyntaxCategory(t *testing.T) {
	for name, decode := range map[string]func([]byte) error{
		"tagged": func(b []byte) error {
			_, err := runner.DecodeMessage(b)
			return err
		},
		"untagged": func(b []byte) error {
			_, err := unions.DecodeArtifactLocation(b)
			return err
		},
	} {
		t.Run(name, func(t *testing.T) {
			for _, input := range []string{
				"not json",
				`{"type":`,
				"",
				`{"x":[}`,
				`"\uD800"`,
				"\"\xff\"",
				`{} {}`,
				`{} trailing`,
				strings.Repeat("[", 128) + "0" + strings.Repeat("]", 128),
			} {
				err := decode([]byte(input))
				if !errors.Is(err, contract.ErrSyntax) {
					t.Fatalf("%q: expected syntax category: %v", input, err)
				}
				if !errors.Is(contract.At("frame", err), contract.ErrSyntax) {
					t.Fatal("field wrapping lost syntax category")
				}
			}
			for _, input := range []string{
				`[1,2]`,
				`{"type":"nope"}`,
				`null`,
				`{"type":"ping","extra":true}`,
				`{"type":"ping","type":"ping"}`,
			} {
				err := decode([]byte(input))
				if err == nil || errors.Is(err, contract.ErrSyntax) {
					t.Fatalf("%s: expected invalid value: %v", input, err)
				}
			}
		})
	}
}

func TestNamedMapKeys(t *testing.T) {
	text := `{"values":{"key_a":null,"key_z":"text"},"ids":{"block-1":true}}`
	value, err := keyed.DecodeRecord([]byte(text))
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := contract.EncodeJSON(value)
	if err != nil || string(encoded) != text {
		t.Fatalf("JSON: %s %v", encoded, err)
	}
	packed, err := value.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	restored, err := keyed.DecodeRecordMsgpack(packed)
	if err != nil || !reflect.DeepEqual(value, restored) {
		t.Fatalf("MessagePack: %+v %v", restored, err)
	}
	again, err := restored.MarshalMsgpack()
	if err != nil || !bytes.Equal(packed, again) {
		t.Fatal("map order changed")
	}
	for _, tc := range []struct{ old, new string }{
		{"key_a", "bad"}, {"key_a", "key_reserved"}, {"block-1", ""},
	} {
		input := strings.Replace(text, tc.old, tc.new, 1)
		_, err := keyed.DecodeRecord([]byte(input))
		if err == nil || errors.Is(err, contract.ErrSyntax) {
			t.Fatalf("key validation: %s: %v", input, err)
		}
		// Produce invalid wire input without using the generated validating encoder.
		raw, err := contract.JSON([]byte(input))
		if err != nil {
			t.Fatal(err)
		}
		data, err := contract.EncodeMsgpack(raw)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := keyed.DecodeRecordMsgpack(data); err == nil {
			t.Fatalf("accepted invalid MessagePack key: %s", input)
		}
	}
	value.Values = map[keyed.Key]*string{"key_reserved": nil}
	if _, err := contract.EncodeJSON(value); err == nil || errors.Is(err, contract.ErrSyntax) {
		t.Fatalf("JSON encode skipped key check: %v", err)
	}
	if _, err := value.MarshalMsgpack(); err == nil {
		t.Fatal("MessagePack encode skipped key check")
	}
	for _, key := range []keyed.Key{"bad", "key_reserved"} {
		value := keyed.NamedMap{key: true}
		if err := value.Validate(); err == nil {
			t.Fatal("named map skipped key validation")
		}
	}
	good := keyed.NamedMap{"key_a": true}
	data, err := good.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := keyed.DecodeNamedMapMsgpack(data); err != nil {
		t.Fatal(err)
	}
}

// Rust's Shape::Record emitter and the reference Failures use z.string() keys.
// A package load costs about a second; no JS installation or network is needed.
func TestNamedMapKeyZodFidelity(t *testing.T) {
	dir := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/keyed"}, true, dir, false); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(dir, "plugin-keyed", "plugin.ts"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), `"values": z.record(z.string(), z.string().nullable())`) ||
		!strings.Contains(string(data), `"ids": z.record(z.string(), z.boolean())`) {
		t.Fatalf("record differs from Rust emitter: %s", data)
	}
}
