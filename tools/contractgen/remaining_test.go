package main

import (
	"encoding/binary"
	"encoding/json"
	"math"
	"strconv"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	wire "github.com/wspl/demi/tools/contractgen/testdata/remaining"
)

// Port of replies_name_their_operation_before_its_result for JSON and MessagePack.
func TestAdjacentReplyRefusals(t *testing.T) {
	cases := []string{
		`{"type":"fs_ok","id":"fs","result":true,"op":"exists"}`,
		`{"type":"fs_ok","id":"fs","op":"unlink","result":null}`,
		`{"type":"fs_ok","id":"fs","op":"mkdir","result":true}`,
		`{"type":"fs_ok","id":"fs","op":"exists"}`,
		`{"type":"fs_ok","id":"fs","op":"exists","result":true,"extra":1}`,
		`{"type":"git_ok","id":"git","op":"show","result":{}}`,
		`{"type":"git_ok","id":"git","op":"show","result":[]}`,
		`{"type":"fs_ok","id":"fs","op":"exists","result":null}`,
		`{"type":"fs_ok","op":"exists","result":true}`,
	}
	for _, text := range cases {
		t.Run(text, func(t *testing.T) {
			if _, err := wire.DecodeMessage([]byte(text)); err == nil {
				t.Fatal("accepted invalid JSON reply")
			}
			data, err := contract.EncodeMsgpack(json.RawMessage(text))
			if err != nil {
				t.Fatal(err)
			}
			if _, err := wire.DecodeMessageMsgpack(data); err == nil {
				t.Fatal("accepted invalid MessagePack reply")
			}
		})
	}
	// IDs can come after content; only op must come before result.
	text := `{"type":"fs_ok","op":"exists","result":true,"id":"fs"}`
	value, err := wire.DecodeMessage([]byte(text))
	if err != nil {
		t.Fatal(err)
	}
	got, err := contract.EncodeJSON(value)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != `{"type":"fs_ok","id":"fs","op":"exists","result":true}` {
		t.Fatalf("wrong field order: %s", got)
	}
	for _, text := range []string{`{"op":"exists","result":true}`, `{"op":"mkdir","result":null}`} {
		value, err := wire.DecodeFsResult([]byte(text))
		if err != nil {
			t.Fatal(err)
		}
		got, err := contract.EncodeJSON(value)
		if err != nil || string(got) != text {
			t.Fatalf("standalone union: %s %v", got, err)
		}
	}
	for _, text := range []string{
		`{"op":"exists","op":"exists","result":true}`,
		`{"op":"exists","result":true,"result":false}`,
	} {
		if _, err := wire.DecodeFsResult([]byte(text)); err == nil {
			t.Fatal("accepted duplicate adjacent key")
		}
	}
}

func TestRunnerTimestampForms(t *testing.T) {
	for _, tc := range []struct {
		value int64
		size  int
	}{
		{
			0,
			6,
		}, {
			4294967295000,
			6,
		}, {
			1,
			10,
		}, {
			4294967296000,
			10,
		}, {
			17179869183999,
			10,
		}, {
			17179869184000,
			15,
		}, {
			-1,
			15,
		}, {
			math.MaxInt64,
			15,
		}, {
			-9223372036854775000,
			15,
		},
	} {
		t.Run(strconv.FormatInt(tc.value, 10), func(t *testing.T) {
			data, err := wire.Timestamp(tc.value).MarshalMsgpack()
			if err != nil {
				t.Fatal(err)
			}
			if len(data) != tc.size {
				t.Fatalf("length %d, want %d", len(data), tc.size)
			}
			got, err := wire.DecodeTimestampMsgpack(data)
			if err != nil || int64(got) != tc.value {
				t.Fatalf("got %d, %v", got, err)
			}
		})
	}
	ext := func(seconds int64, nanos uint32) []byte {
		b := make([]byte, 15)
		copy(b, []byte{0xc7, 12, 0xff})
		binary.BigEndian.PutUint32(b[3:], nanos)
		binary.BigEndian.PutUint64(b[7:], uint64(seconds))
		return b
	}
	for _, tc := range []struct {
		seconds int64
		nanos   uint32
		want    int64
	}{{1, 123999999, 1123}, {-1, 999999999, -1}} {
		got, err := wire.DecodeTimestampMsgpack(ext(tc.seconds, tc.nanos))
		if err != nil || int64(got) != tc.want {
			t.Fatalf("truncation: %d %v", got, err)
		}
	}
	for _, data := range [][]byte{
		{0xd6, 0, 0, 0, 0, 0},
		{0xd4, 0xff, 0},
		{0xd6, 0xff, 0, 0, 0},
		{0xd6, 0xff, 0, 0, 0, 0, 0},
		{0},
		{0xc0},
		ext(0, 1000000000),
		ext(math.MaxInt64, 0),
		ext(math.MinInt64, 0),
		ext(math.MaxInt64/1000, 808000000),
		ext(math.MinInt64/1000-1, 999000000),
	} {
		if _, err := wire.DecodeTimestampMsgpack(data); err == nil {
			t.Fatalf("accepted invalid timestamp %x", data)
		}
	}
}

func TestOpaqueRunnerJSON(t *testing.T) {
	text := `{"z":[null,true,false,"<>&\u2028\u2029",-9223372036854775808,18446744073709551615,1.25,1.0,-0.0],"a":{}}`
	value := &wire.Manifest{Manifest: json.RawMessage(text)}
	data, err := wire.EncodeMessageMsgpack(value)
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := wire.DecodeMessageMsgpack(data)
	if err != nil {
		t.Fatal(err)
	}
	want := strings.ReplaceAll(strings.ReplaceAll(text, `\u2028`, "\u2028"), `\u2029`, "\u2029")
	if got := string(decoded.(*wire.Manifest).Manifest); got != want {
		t.Fatalf("opaque value changed\ngot  %s\nwant %s", got, want)
	}
	for _, raw := range [][]byte{
		{
			0xc4,
			0,
		},
		{
			0xd6,
			0xff,
			0,
			0,
			0,
			0,
		},
		{
			0x81,
			1,
			2,
		},
		{
			0x82,
			0xa1,
			'a',
			1,
			0xa1,
			'a',
			2,
		},
		{
			0xc0,
			0xc0,
		},
	} {
		if _, err := contract.MsgpackJSON(raw); err == nil {
			t.Fatalf("accepted non-JSON MessagePack %x", raw)
		}
	}
}

func TestAdjacentSchema(t *testing.T) {
	var schema struct {
		Properties map[string]json.RawMessage `json:"properties"`
		AllOf      []struct {
			OneOf []struct {
				Properties           map[string]json.RawMessage `json:"properties"`
				AdditionalProperties *bool                      `json:"additionalProperties"`
			} `json:"oneOf"`
		} `json:"allOf"`
	}
	if err := json.Unmarshal(wire.FsOKJSONSchema(), &schema); err != nil {
		t.Fatal(err)
	}
	if len(schema.Properties) != 4 || len(schema.AllOf) != 1 || len(schema.AllOf[0].OneOf) != 16 {
		t.Fatalf("flattened schema: %s", wire.FsOKJSONSchema())
	}
	for _, branch := range schema.AllOf[0].OneOf {
		if len(branch.Properties) != 2 || branch.Properties["op"] == nil || branch.Properties["result"] == nil ||
			branch.AdditionalProperties != nil {
			t.Fatalf("invalid adjacent branch: %+v", branch)
		}
	}
	var timestamp map[string]any
	if err := json.Unmarshal(wire.TimestampJSONSchema(), &timestamp); err != nil {
		t.Fatal(err)
	}
	if timestamp["type"] != "integer" || timestamp["format"] != "int64" {
		t.Fatalf("timestamp schema: %v", timestamp)
	}
}
