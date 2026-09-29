package wiregen

import (
	"bytes"
	"encoding/hex"
	"encoding/json/jsontext"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/tinylib/msgp/msgp"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/internal/wiregen/packtest"
)

// The generated codec is exercised at its entry points; this table takes under
// one second and uses no processes, timers or external resources.
func TestMessagePackContracts(t *testing.T) {
	record := packtest.Record{Name: "a", Small: 255, Data: []byte{0, 255}, At: -1, Items: []packtest.Item{{Kind: "yes"}}, Labels: map[string]*string{"unset": nil}, Raw: jsontext.Value(`{"n":18446744073709551615}`), Event: packtest.Changed{Count: 1}, Choice: packtest.Text("word"), Call: packtest.Run{Limit: 1}}
	data, err := record.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := packtest.DecodeRecordMsgpack(data)
	if err != nil {
		t.Fatal(err)
	}
	again, err := decoded.MarshalMsgpack()
	if err != nil || !bytes.Equal(data, again) {
		t.Fatalf("round trip: %x %v", again, err)
	}
	compare(t, "testdata/msgpack/record.hex", []byte(hex.EncodeToString(data)+"\n"))
	fields, err := wire.MPObject(data)
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name, field string
		value       []byte
		want        string
	}{
		{"missing", "name", nil, "name: required"},
		{"nonfinite", "ratio", msgp.AppendFloat64(nil, math.NaN()), "ratio: expected finite number"},
		{"null optional", "optional", msgp.AppendNil(nil), "optional:"},
		{"required nullable", "nullable", nil, "nullable: required"},
		{"overflow", "small", msgp.AppendUint64(nil, 256), "small:"},
		{"binary disguised", "data", msgp.AppendString(nil, "secret"), "data: expected binary bytes"},
		{"timestamp disguised", "at", msgp.AppendInt64(nil, 0), "at:"},
		{"unknown", "extra", msgp.AppendBool(nil, true), "extra: unknown member"},
		{"rule", "name", msgp.AppendString(nil, ""), "name:"},
		{"nested rule", "items", []byte{0x91, 0x81, 0xa4, 'k', 'i', 'n', 'd', 0xa3, 'b', 'a', 'd'}, "items[0].kind:"},
		{"nested structure", "items", []byte{0x91, 0x80}, "items[0].kind: required"},
		{"unknown variant", "event", []byte{0x81, 0xa4, 't', 'y', 'p', 'e', 0xa1, 'x'}, "event.type:"},
		{"adjacent path", "call", []byte{0x82, 0xa2, 'o', 'p', 0xa3, 'r', 'u', 'n', 0xa6, 'p', 'a', 'r', 'a', 'm', 's', 0x80}, "call.params.limit: required"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			var changed []wire.MPMember
			found := false
			for _, field := range fields {
				if field.Name == c.field {
					found = true
					if c.value != nil {
						changed = append(changed, wire.MPMember{Name: field.Name, Data: c.value})
					}
				} else {
					changed = append(changed, field)
				}
			}
			if !found && c.value != nil {
				changed = append(changed, wire.MPMember{Name: c.field, Data: c.value})
			}
			raw := msgp.AppendMapHeader(nil, uint32(len(changed)))
			for _, field := range changed {
				raw = msgp.AppendString(raw, field.Name)
				raw = append(raw, field.Data...)
			}
			_, err := packtest.DecodeRecordMsgpack(raw)
			if err == nil || !strings.Contains(err.Error(), c.want) || strings.Contains(err.Error(), "secret") {
				t.Fatalf("got %v, want %s", err, c.want)
			}
		})
	}
	for _, raw := range [][]byte{append(bytes.Clone(data), 0), {0xdf, 255, 255, 255, 255}} {
		if _, err := packtest.DecodeRecordMsgpack(raw); err == nil {
			t.Fatalf("accepted malformed map %x", raw)
		}
	}
	duplicate := msgp.AppendMapHeader(nil, uint32(len(fields)+1))
	for _, field := range fields {
		duplicate = msgp.AppendString(duplicate, field.Name)
		duplicate = append(duplicate, field.Data...)
	}
	duplicate = msgp.AppendString(duplicate, "name")
	duplicate = msgp.AppendString(duplicate, "other-valid-name")
	if _, err := packtest.DecodeRecordMsgpack(duplicate); err == nil || !strings.Contains(err.Error(), "name: duplicate member") {
		t.Fatalf("duplicate in otherwise valid record: %v", err)
	}
	record.Name = ""
	if _, err := record.MarshalMsgpack(); err == nil {
		t.Fatal("encoded a broken rule")
	}
}

func TestMessagePackGeneratorGolden(t *testing.T) {
	pkg, err := Load("packtest")
	if err != nil {
		t.Fatal(err)
	}
	files, err := pkg.GenerateGo()
	if err != nil {
		t.Fatal(err)
	}
	compare(t, "testdata/msgpack/output.golden", files["msgpack_wire.go"])
	// Turning on MessagePack leaves each existing JSON output byte-identical.
	pkg.MessagePack = nil
	previous, err := pkg.GenerateGo()
	if err != nil {
		t.Fatal(err)
	}
	for name, data := range previous {
		if !bytes.Equal(data, files[name]) {
			t.Errorf("JSON output changed: %s", name)
		}
	}
}

func TestMessagePackTimestamps(t *testing.T) {
	for _, data := range [][]byte{{0xc8, 0, 4, 255, 0, 0, 0, 1}, {0xc9, 0, 0, 0, 4, 255, 0, 0, 0, 1}} {
		got, err := wire.MPTimestamp(data)
		if err != nil || got != 1000 {
			t.Fatalf("nonminimal timestamp header: %d %v", got, err)
		}
	}

	for _, value := range []int64{0, 1, -1, 4294967295000, 4294967295001, 17179869184000, 9223372036854775807} {
		data := wire.MPAppendTimestamp(nil, value)
		got, err := wire.MPTimestamp(data)
		if err != nil || got != value {
			t.Fatalf("timestamp %d: %d, %v", value, got, err)
		}
	}
	for _, data := range [][]byte{{0xd4, 0xff, 0}, {0xd6, 0, 0, 0, 0, 0}, {0xd7, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0}} {
		if _, err := wire.MPTimestamp(data); err == nil {
			t.Fatalf("accepted invalid timestamp %x", data)
		}
	}
}

func TestMessagePackOptInRefusesUnknownRepresentation(t *testing.T) {
	source, err := os.ReadFile(filepath.Join("packtest", "contract.go"))
	if err != nil {
		t.Fatal(err)
	}
	source = bytes.Replace(source, []byte(`msgpack:"bin"`), []byte(`msgpack:"other"`), 1)
	pkg, err := LoadSource(map[string][]byte{"contract.go": source})
	if err != nil {
		t.Fatal(err)
	}
	if _, err = pkg.GenerateGo(); err == nil {
		t.Fatal("unknown representation was accepted")
	}
}

func TestMessagePackUnionEnvelopes(t *testing.T) {
	envelope := packtest.Envelope{ID: "i", Call: packtest.Run{Limit: 2}}
	data, err := envelope.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := packtest.DecodeEnvelopeMsgpack(data)
	if err != nil {
		t.Fatal(err)
	}
	again, err := decoded.MarshalMsgpack()
	if err != nil || !bytes.Equal(data, again) {
		t.Fatalf("inline union: %x %v", again, err)
	}
	// A content member unknown to an open variant is ignored.
	open := []byte{0x82, 0xa2, 'o', 'p', 0xa3, 'r', 'u', 'n', 0xa6, 'p', 'a', 'r', 'a', 'm', 's', 0x82, 0xa5, 'l', 'i', 'm', 'i', 't', 1, 0xa5, 'e', 'x', 't', 'r', 'a', 0xc0}
	if _, err := packtest.DecodeCallMsgpack(open); err != nil {
		t.Fatal(err)
	}
	bad := bytes.Replace(open, []byte{0xa5, 'l', 'i', 'm', 'i', 't', 1}, []byte{0xa5, 'l', 'i', 'm', 'i', 't', 0}, 1)
	if _, err := packtest.DecodeCallMsgpack(bad); err == nil || !strings.Contains(err.Error(), "params.limit") {
		t.Fatalf("content rule path: %v", err)
	}
	if _, err := packtest.DecodeChoiceMsgpack(msgp.AppendBool(nil, true)); err == nil {
		t.Fatal("unmatched untagged value accepted")
	}
	var absent *packtest.Changed
	if _, err := packtest.EncodeEventMsgpack(absent); err == nil {
		t.Fatal("typed nil accepted")
	}
}

func TestMessagePackFloatReadsIntegerPrefixes(t *testing.T) {
	for _, data := range [][]byte{msgp.AppendInt64(nil, -1), msgp.AppendUint64(nil, 1), msgp.AppendFloat64(nil, 1)} {
		if _, err := wire.MPFloat(data); err != nil {
			t.Fatal(err)
		}
	}
}
