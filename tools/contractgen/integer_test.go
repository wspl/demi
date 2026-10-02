package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/tools/contractgen/testdata/integers"
)

// NumberVisitor in crates/agent-tools/src/input.rs delegates strings to
// u64::from_str. These scenarios protect that boundary in both formats.
// Budget: one second, no external resources.
func TestIntegerStrings(t *testing.T) {
	for _, tc := range []struct {
		text  string
		valid bool
		value uint64
	}{
		{"0", true, 0}, {"17", true, 17}, {"+17", true, 17}, {"00017", true, 17}, {"+000", true, 0},
		{"18446744073709551615", true, ^uint64(0)},
		{"18446744073709551616", false, 0}, {"-0", false, 0}, {"-1", false, 0},
		{"", false, 0}, {"+", false, 0}, {"++1", false, 0}, {" 1", false, 0}, {"1 ", false, 0},
		{"\t1", false, 0}, {"1\n", false, 0}, {"1_0", false, 0}, {"0x10", false, 0},
		{"1.0", false, 0}, {"1e2", false, 0}, {"１２", false, 0},
	} {
		t.Run(tc.text, func(t *testing.T) {
			for _, packed := range []bool{false, true} {
				encode := contract.EncodeJSON
				decode := integers.DecodeInput
				if packed {
					encode = contract.EncodeMsgpack
					decode = integers.DecodeInputMsgpack
				}
				raw, err := encode(struct {
					CommandID string `json:"commandId" msgpack:"commandId"`
					ShellID   string `json:"shellId" msgpack:"shellId"`
				}{tc.text, tc.text})
				if err != nil {
					t.Fatal(err)
				}
				value, err := decode(raw)
				if (err == nil) != tc.valid {
					t.Fatalf("packed=%v: %q: %v", packed, raw, err)
				}
				if !tc.valid {
					continue
				}
				if value.CommandID != tc.value || value.ShellID == nil || uint64(*value.ShellID) != tc.value {
					t.Fatalf("wrong IDs: %+v", value)
				}
				got, err := encode(value)
				if err != nil {
					t.Fatal(err)
				}
				want, err := encode(struct {
					CommandID uint64 `json:"commandId" msgpack:"commandId"`
					ShellID   uint64 `json:"shellId" msgpack:"shellId"`
				}{tc.value, tc.value})
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(got, want) {
					t.Fatalf("encoding %x != %x", got, want)
				}
			}
		})
	}
	for _, raw := range []string{`{"commandId":null}`, `{"commandId":1,"shellId":null}`, `{"commandId":1.0}`, `{"commandId":-1}`, `{"commandId":18446744073709551616}`} {
		if _, err := integers.DecodeInput([]byte(raw)); err == nil {
			t.Fatalf("accepted %s", raw)
		}
	}
	if _, err := integers.DecodeInput([]byte(`{"commandId":17}`)); err != nil {
		t.Fatal(err)
	}
	for _, raw := range []string{`"-5"`, `"+5"`, `5`} {
		if _, err := integers.DecodeSmall([]byte(raw)); err != nil {
			t.Fatal(err)
		}
	}
	for _, raw := range []string{`"-6"`, `"6"`, `"128"`, `"-129"`, `"+-1"`, `null`} {
		if _, err := integers.DecodeSmall([]byte(raw)); err == nil {
			t.Fatalf("accepted %s", raw)
		}
	}
}

// The Rust tool schema test asserts integer shellId; the tool caller removes
// the root title (the generator retains it for other schema consumers).
// The captured schemars 1.2.2 reference is testdata/integers/rust-schema.json.
// Budget: five seconds for generation; no network.
func TestIntegerSchema(t *testing.T) {
	raw, err := os.ReadFile("testdata/integers/rust-schema.json")
	if err != nil {
		t.Fatal(err)
	}
	var reference any
	if err := json.Unmarshal(raw, &reference); err != nil {
		t.Fatal(err)
	}
	want, err := contract.EncodeJSON(reference)
	if err != nil {
		t.Fatal(err)
	}
	if got := integers.InputJSONSchema(); !bytes.Equal(got, want) {
		t.Fatalf("schema: %s; Rust: %s", got, want)
	}
	dest := t.TempDir()
	if err := generate(t.Context(), []string{"./testdata/integers"}, true, dest, false); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filepath.Join(dest, "protocol", "contracts.ts"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "export const smallSchema = z.int().min(-128).max(127).min(-5).max(5)\n") {
		t.Fatalf("integer Zod schema changed: %s", data)
	}
}
