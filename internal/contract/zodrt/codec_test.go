package zodrt

import (
	"bytes"
	"encoding/hex"
	"math"
	"testing"
	"time"
)

// JSON.stringify's output for these values, taken from Bun.
func TestEncodeJSONMatchesJavaScript(t *testing.T) {
	cases := []struct {
		value any
		want  string
	}{
		{"<a & b>", `"<a & b>"`},
		{"line sep ", "\"line sep \""},
		{"\x00\x01\b\f\n\r\t\x1f\"\\", `"\u0000\u0001\b\f\n\r\t\u001f\"\\"`},
		{"😀", `"😀"`},
		{math.Copysign(0, -1), `0`},
		{1e21, `1e+21`},
		{1e-7, `1e-7`},
		{0.1, `0.1`},
		{5e-324, `5e-324`},
		{float64(1 << 60), `1152921504606847000`},
		{int64(-42), `-42`},
		{Object{{Key: "b", Value: nil}, {Key: "a", Value: []any{true, false}}}, `{"b":null,"a":[true,false]}`},
		{map[string]any{"b": 1.0, "a": 2.0}, `{"a":2,"b":1}`},
	}
	for _, test := range cases {
		got, err := EncodeJSON(test.value, false)
		if err != nil {
			t.Fatalf("%v: %v", test.value, err)
		}
		if string(got) != test.want {
			t.Errorf("%#v: got %s, want %s", test.value, got, test.want)
		}
	}
}

func TestEncodeJSONRefusesWhatPlainJSONCannotCarry(t *testing.T) {
	for _, value := range []any{[]byte{1}, time.Unix(0, 0), math.NaN()} {
		if _, err := EncodeJSON(value, false); err == nil {
			t.Errorf("%#v: encoded", value)
		}
	}
}

// stringifyPortableJson's markers, and parsePortableJson's revival of them.
func TestPortableJSON(t *testing.T) {
	instant := time.UnixMilli(1_700_000_000_123).UTC()
	value := Object{{Key: "data", Value: []byte{1, 2, 3}}, {Key: "at", Value: instant}}
	encoded, err := EncodeJSON(value, true)
	if err != nil {
		t.Fatal(err)
	}
	want := `{"data":{"__demiUint8Array":true,"base64":"AQID"},"at":{"__demiDate":true,"iso":"2023-11-14T22:13:20.123Z"}}`
	if string(encoded) != want {
		t.Fatalf("got %s, want %s", encoded, want)
	}
	decoded, err := DecodeJSON(encoded, true)
	if err != nil {
		t.Fatal(err)
	}
	fields := decoded.(map[string]any)
	if !bytes.Equal(fields["data"].([]byte), []byte{1, 2, 3}) || !fields["at"].(time.Time).Equal(instant) {
		t.Fatalf("revived %#v", decoded)
	}
	extended := time.Date(-271821, time.April, 20, 0, 0, 0, 0, time.UTC)
	if got := isoString(extended); got != "-271821-04-20T00:00:00.000Z" {
		t.Fatalf("extended year %s", got)
	}
	if parsed, err := parseISOString("-271821-04-20T00:00:00.000Z"); err != nil || !parsed.Equal(extended) {
		t.Fatalf("parse extended year: %v %v", parsed, err)
	}
	for _, text := range []string{"2023-02-30T00:00:00.000Z", "2023-11-14T22:13:20Z", "2023-11-14"} {
		if _, err := parseISOString(text); err == nil {
			t.Errorf("%s: parsed", text)
		}
	}
}

// @msgpack/msgpack's output for these values, taken from Bun.
func TestEncodeMsgpackMatchesJavaScript(t *testing.T) {
	cases := []struct {
		value any
		want  string
	}{
		{float64(0), "00"},
		{math.Copysign(0, -1), "00"},
		{float64(-33), "d0df"},
		{float64(255), "ccff"},
		{float64(1<<53 - 1), "cf001fffffffffffff"},
		{float64(1 << 53), "cb4340000000000000"},
		{1.5, "cb3ff8000000000000"},
		{int64(-40000), "d2ffff63c0"},
		{[]byte(nil), "c400"},
		{"é", "a2c3a9"},
		{time.Unix(1, 0), "d6ff00000001"},
		{time.UnixMilli(1_700_000_000_123), "d7ff1d5353006553f100"},
		{time.UnixMilli(-1_500), "c70cff1dcd6500fffffffffffffffe"},
		{Object{{Key: "b", Value: nil}, {Key: "a", Value: true}}, "82a162c0a161c3"},
	}
	for _, test := range cases {
		got, err := EncodeMsgpack(test.value)
		if err != nil {
			t.Fatalf("%v: %v", test.value, err)
		}
		if hex.EncodeToString(got) != test.want {
			t.Errorf("%#v: got %x, want %s", test.value, got, test.want)
		}
	}
}

func TestDecodeMsgpack(t *testing.T) {
	decode := func(text string) any {
		t.Helper()
		data, err := hex.DecodeString(text)
		if err != nil {
			t.Fatal(err)
		}
		value, err := DecodeMsgpack(data)
		if err != nil {
			t.Fatalf("%s: %v", text, err)
		}
		return value
	}
	if got := decode("cf001fffffffffffff"); got != float64(1<<53-1) {
		t.Errorf("uint64 decoded as %#v", got)
	}
	if got := decode("ca3fc00000"); got != 1.5 {
		t.Errorf("float32 decoded as %#v", got)
	}
	// new Date(-2 * 1000 + 999999999 / 1e6) truncates toward zero.
	if got := decode("c70cff3b9ac9fffffffffffffffffe").(time.Time); got.UnixMilli() != -1000 {
		t.Errorf("timestamp decoded as %v", got.UnixMilli())
	}
	if _, err := DecodeMsgpack([]byte{0x01, 0x02}); err == nil {
		t.Error("extra bytes accepted")
	}
}

func TestCheckURL(t *testing.T) {
	for _, text := range []string{"https://example.com/a?b#c", "  http://localhost:8080/ ", "mailto:someone@example.com"} {
		if err := CheckURL(text); err != nil {
			t.Errorf("%q: %v", text, err)
		}
	}
	for _, text := range []string{"not a url", "https://exa mple.com", "/relative"} {
		if err := CheckURL(text); err == nil {
			t.Errorf("%q: accepted", text)
		}
	}
}

func TestFieldsAndNumbers(t *testing.T) {
	value := map[string]any{"a": 1.0, "extra": true}
	if _, err := Fields(value, Strict, []string{"a"}); err == nil {
		t.Error("strict object accepted an unknown key")
	}
	if _, err := Fields(value, Strip, []string{"a"}); err != nil {
		t.Errorf("stripping object: %v", err)
	}
	if _, err := Int(float64(1 << 53)); err == nil {
		t.Error("unsafe integer accepted")
	}
	if _, err := Int(1.5); err == nil {
		t.Error("fraction accepted as an integer")
	}
	if _, err := Number(math.Inf(1)); err == nil {
		t.Error("infinity accepted")
	}
	if Length("😀😀") != 2 {
		t.Error("length is not counted in code points")
	}
	if err := At("a", AtIndex(2, Invalid("bad"))); err.Error() != "a.2: bad" {
		t.Errorf("path %q", err)
	}
}
