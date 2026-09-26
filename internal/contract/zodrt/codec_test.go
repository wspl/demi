package zodrt

import (
	"bytes"
	"encoding/hex"
	"math"
	"runtime"
	"strings"
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
		{Object{{Key: "b", Value: 1.0}, {Key: "10", Value: 2.0}, {Key: "a", Value: 3.0}, {Key: "2", Value: 4.0}, {Key: "01", Value: 5.0}}, `{"2":4,"10":2,"b":1,"a":3,"01":5}`},
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
	fields := decoded.(Object)
	if !bytes.Equal(fields[0].Value.([]byte), []byte{1, 2, 3}) || !fields[1].Value.(time.Time).Equal(instant) {
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
		{time.Unix(1, 999_999), "d6ff00000001"},
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

// A key repeated in the text keeps its first position and takes its last
// value, as JSON.parse and @msgpack/msgpack leave it.
func TestDecodersKeepKeyOrder(t *testing.T) {
	want := Object{{Key: "b", Value: 3.0}, {Key: "a", Value: 2.0}}
	fromJSON, err := DecodeJSON([]byte(`{"b":1,"a":2,"b":3}`), false)
	if err != nil {
		t.Fatal(err)
	}
	fromMsgpack, err := DecodeMsgpack([]byte{0x83, 0xa1, 'b', 0x01, 0xa1, 'a', 0x02, 0xa1, 'b', 0x03})
	if err != nil {
		t.Fatal(err)
	}
	for _, got := range []any{fromJSON, fromMsgpack} {
		fields := got.(Object)
		if len(fields) != 2 || fields[0] != want[0] || fields[1] != want[1] {
			t.Errorf("decoded %#v", got)
		}
	}
}

// Nesting beyond MaxDepth is refused before it is walked; a frame of nested
// arrays would otherwise exhaust the stack.
func TestDecodersRefuseDeepNesting(t *testing.T) {
	deep := func(open, close string, depth int) []byte {
		return []byte(strings.Repeat(open, depth) + "0" + strings.Repeat(close, depth))
	}
	if _, err := DecodeJSON(deep("[", "]", MaxDepth), false); err != nil {
		t.Errorf("JSON at the limit: %v", err)
	}
	if _, err := DecodeJSON(deep("[", "]", MaxDepth+1), false); err == nil {
		t.Error("JSON beyond the limit decoded")
	}
	if _, err := DecodeJSON(deep(`{"a":`, "}", MaxDepth+1), false); err == nil {
		t.Error("JSON objects beyond the limit decoded")
	}
	atLimit := append(bytes.Repeat([]byte{0x91}, MaxDepth), 0xc0)
	if _, err := DecodeMsgpack(atLimit); err != nil {
		t.Errorf("MessagePack at the limit: %v", err)
	}
	// Four million nested arrays, within a runner frame.
	huge := append(bytes.Repeat([]byte{0x91}, 4<<20-1), 0xc0)
	if _, err := DecodeMsgpack(huge); err == nil {
		t.Error("MessagePack beyond the limit decoded")
	}
	maps := append(bytes.Repeat([]byte{0x81, 0xa1, 'a'}, MaxDepth+1), 0xc0)
	if _, err := DecodeMsgpack(maps); err == nil {
		t.Error("MessagePack maps beyond the limit decoded")
	}
}

// TypeScript never writes these; a decoder refuses them instead of repairing
// them.
func TestDecodersRefuseMalformedText(t *testing.T) {
	for _, text := range []string{`"\ud800"`, `"\udc00"`, `"\ud800\u0041"`, "\"\xff\""} {
		if _, err := DecodeJSON([]byte(text), false); err == nil {
			t.Errorf("JSON %s decoded", text)
		}
	}
	if _, err := DecodeJSON([]byte(`"\ud83d\ude00 \\ud800"`), false); err != nil {
		t.Errorf("a surrogate pair and an escaped backslash: %v", err)
	}
	for _, frame := range [][]byte{
		{0xa1, 0xff},
		{0x81, 0xa1, 0xff, 0xc0},
		{0x81, 0x01, 0xc0},
		{0x81, 0xc4, 0x01, 'a', 0xc0},
	} {
		if _, err := DecodeMsgpack(frame); err == nil {
			t.Errorf("MessagePack %x decoded", frame)
		}
	}
}

// Headers may claim 2^32-1 entries at every level; the memory a decoder
// reserves must follow the bytes that arrive, not the claims.
func TestDecodeMsgpackIgnoresClaimedLengths(t *testing.T) {
	var frame []byte
	for range 240 {
		frame = append(frame, 0xdd, 0xff, 0xff, 0xff, 0xff)
	}
	for range 240 {
		frame = append(frame, 0xdf, 0xff, 0xff, 0xff, 0xff, 0xa1, 'k')
	}
	var before, after runtime.MemStats
	runtime.GC()
	runtime.ReadMemStats(&before)
	if _, err := DecodeMsgpack(frame); err == nil {
		t.Fatal("a truncated frame decoded")
	}
	runtime.ReadMemStats(&after)
	if allocated := after.TotalAlloc - before.TotalAlloc; allocated > 256*uint64(len(frame)) {
		t.Errorf("decoding %d bytes allocated %d bytes", len(frame), allocated)
	}
}

// A copy of a record never sees a change made through another copy.
func TestRecordCopiesDoNotAlias(t *testing.T) {
	var original Record[string, int]
	original.Set("a", 1)
	original.Set("b", 2)
	copied := original
	copied.Set("c", 3)
	copied.Set("a", 10)
	if _, ok := original.Get("c"); ok || original.Len() != 2 {
		t.Errorf("a new key leaked into the original: %d keys", original.Len())
	}
	if value, _ := original.Get("a"); value != 1 {
		t.Errorf("an overwrite leaked into the original: %d", value)
	}
	original.Set("d", 4)
	if _, ok := copied.Get("d"); ok {
		t.Error("the original's new key leaked into the copy")
	}
	if value, _ := copied.Get("c"); value != 3 {
		t.Errorf("the copy's key was overwritten: %d", value)
	}
	parsed, err := ParseRecord(String, Int)(Object{{Key: "x", Value: 1.0}})
	if err != nil {
		t.Fatal(err)
	}
	other := parsed
	other.Set("y", 2)
	other.Set("x", 5)
	if value, _ := parsed.Get("x"); value != 1 || parsed.Len() != 1 {
		t.Error("a decoded record shares changes with its copy")
	}
}

// Portable JSON revives only the forms stringifyPortableJson writes.
func TestPortableJSONRefusesNonCanonicalForms(t *testing.T) {
	for _, text := range []string{
		`{"__demiUint8Array":true,"base64":"AQ ID"}`,
		`{"__demiUint8Array":true,"base64":"AQID\n"}`,
		`{"__demiBigInt":true,"value":"0x10"}`,
		`{"__demiBigInt":true,"value":" 12 "}`,
		`{"__demiBigInt":true,"value":"012"}`,
	} {
		if _, err := DecodeJSON([]byte(text), true); err == nil {
			t.Errorf("%s decoded", text)
		}
	}
}
