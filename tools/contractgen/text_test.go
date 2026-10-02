package main

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/tools/contractgen/testdata/text"
)

// Ports web-api-protocol's text tests through generated boundaries. CPU only,
// below one second; both codecs must canonicalize before parent validation.
func TestFormatEmail(t *testing.T) {
	longest := strings.Repeat("a", 254-13) + "@example.test"
	for _, input := range []string{"  Ana@Example.TEST \n", "a@b.co", "first.last+tag@sub.example.org", "o'neil_x-y@a-b.example", "K@example.test", "  " + strings.ToUpper(longest) + "  "} {
		wire, err := json.Marshal(input)
		if err != nil {
			t.Fatal(err)
		}
		value, err := text.DecodeEmail(wire)
		if err != nil {
			t.Fatalf("%q: %v", input, err)
		}
		want := strings.ToLower(strings.TrimSpace(input))
		if string(value) != want {
			t.Fatalf("%q: got %q, want %q", input, value, want)
		}
		encoded, err := contract.EncodeMsgpack(input)
		if err != nil {
			t.Fatal(err)
		}
		msg, err := text.DecodeEmailMsgpack(encoded)
		if err != nil || msg != value {
			t.Fatalf("MessagePack %q: %q, %v", input, msg, err)
		}
		parsed, err := text.ParseEmail(input)
		if err != nil || parsed != value {
			t.Fatalf("constructor %q: %q, %v", input, parsed, err)
		}
	}
	for _, input := range []string{"a" + longest, "invalid", ".ana@example.test", "ana..b@example.test", "ana.@example.test", "ana@example", "ana@-example.test", "ana@example.t", "ana@exa_mple.test", "an a@example.test", "anä@example.test", "İ@example.test", ""} {
		wire, err := json.Marshal(input)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := text.DecodeEmail(wire); err == nil {
			t.Errorf("accepted %q", input)
		}
	}
	// Direct construction cannot bypass the canonical spelling on output.
	if _, err := json.Marshal(text.Email("Ana@Example.TEST")); err == nil {
		t.Fatal("encoded noncanonical email")
	}
	value, err := text.DecodeReceived([]byte(`{"email":" ANA@example.test "}`))
	if err != nil || value.Email != "ana@example.test" {
		t.Fatalf("nested normalization: %+v, %v", value, err)
	}
}

func TestFormatTrimmed(t *testing.T) {
	for _, tc := range []struct{ input, want string }{
		{"\ufeff  New name \t\u2003", "New name"},
		{"\u0085name", "\u0085name"},
		{" \n" + strings.Repeat("😀", 8) + "\t", strings.Repeat("😀", 8)},
	} {
		wire, err := json.Marshal(tc.input)
		if err != nil {
			t.Fatal(err)
		}
		value, err := text.DecodeName(wire)
		if err != nil || string(value) != tc.want {
			t.Fatalf("%q: %q, %v", tc.input, value, err)
		}
		encoded, err := contract.EncodeMsgpack(tc.input)
		if err != nil {
			t.Fatal(err)
		}
		msg, err := text.DecodeNameMsgpack(encoded)
		if err != nil || msg != value {
			t.Fatalf("MessagePack %q: %q, %v", tc.input, msg, err)
		}
	}
	for _, input := range []string{"", " \t\n\ufeff\u2003", strings.Repeat("😀", 9), "123456789"} {
		wire, err := json.Marshal(input)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := text.DecodeName(wire); err == nil {
			t.Errorf("accepted %q", input)
		}
	}
	if _, err := json.Marshal(text.Name(" x ")); err == nil {
		t.Fatal("encoded untrimmed name")
	}
}
