package runnerwire_test

import (
	"bytes"
	"math"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/runnerwire"
)

// These boundary tests use in-memory records; the suite budget is one second.
func TestKeptCorpus(t *testing.T) {
	data, err := os.ReadFile("testdata/kept/output.msgpack")
	if err != nil {
		t.Fatal(err)
	}
	records, err := runnerwire.DecodeRecords(data)
	if err != nil {
		t.Fatal(err)
	}
	want := []runnerwire.KeptRecord{
		&runnerwire.KeptOutput{Stream: runnerwire.Stdout, Bytes: []byte("first\n")},
		&runnerwire.KeptOutput{Stream: runnerwire.Stderr, Bytes: []byte{0, 255, 10}},
		&runnerwire.KeptLeftOut{Bytes: 734003200},
		&runnerwire.KeptOutput{Stream: runnerwire.Stdout, Bytes: []byte("last\n")},
	}
	if !reflect.DeepEqual(records, want) {
		t.Fatalf("records = %#v", records)
	}
	if encoded := encodeRecords(t, records); !bytes.Equal(encoded, data) {
		t.Fatal("kept corpus bytes changed")
	}
}

func encodeRecords(t *testing.T, records []runnerwire.KeptRecord) []byte {
	t.Helper()
	var data []byte
	for _, record := range records {
		encoded, err := runnerwire.EncodeRecord(record)
		if err != nil {
			t.Fatal(err)
		}
		data = append(data, encoded...)
	}
	return data
}

func TestKeptRecordBound(t *testing.T) {
	gap := encodeRecords(t, []runnerwire.KeptRecord{&runnerwire.KeptLeftOut{Bytes: math.MaxUint64}})
	if runnerwire.JobKeptBytes+len(gap) > runnerwire.JobKeptReadBytes {
		t.Fatal("gap exceeds read allowance")
	}
}

func TestKeptRecordRefusals(t *testing.T) {
	records := []runnerwire.KeptRecord{
		&runnerwire.KeptOutput{Stream: runnerwire.Stdout, Bytes: []byte("a")},
		&runnerwire.KeptLeftOut{Bytes: 7},
		&runnerwire.KeptOutput{Stream: runnerwire.Stderr, Bytes: []byte("b")},
	}
	data := encodeRecords(t, records)
	decoded, err := runnerwire.DecodeRecords(data)
	if err != nil || !reflect.DeepEqual(decoded, records) {
		t.Fatalf("round trip = %v, %v", decoded, err)
	}
	gap := encodeRecords(t, []runnerwire.KeptRecord{&runnerwire.KeptLeftOut{Bytes: 1}})
	empty := encodeRecords(
		t,
		[]runnerwire.KeptRecord{&runnerwire.KeptOutput{Stream: runnerwire.Stdout, Bytes: []byte{}}},
	)
	oversized := encodeRecords(
		t,
		[]runnerwire.KeptRecord{
			&runnerwire.KeptOutput{Stream: runnerwire.Stdout, Bytes: make([]byte, runnerwire.JobKeptBytes)},
		},
	)
	for name, invalid := range map[string][]byte{
		"second gap": append(bytes.Clone(data), gap...),
		"truncated":  data[:len(data)-1],
		"empty read": empty,
		"size":       oversized,
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := runnerwire.DecodeRecords(invalid); err == nil {
				t.Fatal("accepted invalid kept output")
			}
		})
	}
}

func TestReleaseRecord(t *testing.T) {
	record := `{"release":"317dd84e2ce0846a1bea4bc5959959c04af7ba8e4de32b3752fdd6b409f7b1a5",` +
		`"wire":24,"commandProtocol":1,` +
		`"targets":{"aarch64-apple-darwin":` +
		`{"sha256":"dbf18cb3af50a3348a834ea9cee7981f7354f84aa89764f4d68812c81ebe045b","size":38772096},` +
		`"aarch64-unknown-linux-musl":` +
		`{"sha256":"5d4219232ad6a95b2a0e72097011e91ae3773e8523e8032788904fa0e9164197","size":38710848}}}`
	if _, err := runnerwire.DecodeRelease([]byte(record)); err != nil {
		t.Fatal(err)
	}
	for _, change := range [][2]string{
		{"317dd84e", "317DD84E"},
		{`"wire":24`, `"wire":23`},
		{`"commandProtocol":1`, `"commandProtocol":2`},
		{"aarch64-apple-darwin", "aarch64-apple-ios"},
		{`"size":38772096`, `"size":0`},
		{`"wire":24`, `"wire":24,"channel":"beta"`},
	} {
		t.Run(change[1], func(t *testing.T) {
			invalid := strings.Replace(record, change[0], change[1], 1)
			if _, err := runnerwire.DecodeRelease([]byte(invalid)); err == nil {
				t.Fatal("accepted invalid release")
			}
		})
	}
}
