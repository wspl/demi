package main

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/vmihailenco/msgpack/v5"
	"github.com/wspl/demi/tools/contractgen/testdata/kept"
)

// Every stored kept fixture is a stream, so the library only splits records;
// the generated decoder owns every shape check. Budget below one second.
func TestKeptTuple(t *testing.T) {
	fixtures, err := filepath.Glob("testdata/runner-protocol/kept/*")
	if err != nil || len(fixtures) == 0 {
		t.Fatalf("fixtures: %v", err)
	}
	for _, path := range fixtures {
		t.Run(filepath.Base(path), func(t *testing.T) {
			input, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			reader := bytes.NewReader(input)
			decoder := msgpack.NewDecoder(reader)
			var output []byte
			var records []kept.Record
			for reader.Len() > 0 {
				raw, err := decoder.DecodeRaw()
				if err != nil {
					t.Fatal(err)
				}
				value, err := kept.DecodeRecordMsgpack(raw)
				if err != nil {
					t.Fatal(err)
				}
				records = append(records, value)
				encoded, err := kept.EncodeRecordMsgpack(value)
				if err != nil {
					t.Fatal(err)
				}
				output = append(output, encoded...)
			}
			if !bytes.Equal(input, output) {
				t.Fatalf("kept bytes differ: %x", output)
			}
			want := []kept.Record{
				&kept.Output{Stream: "stdout", Data: []byte("first\n")},
				&kept.Output{Stream: "stderr", Data: []byte{0, 255, 10}},
				&kept.LeftOut{Bytes: 734003200},
				&kept.Output{Stream: "stdout", Data: []byte("last\n")},
			}
			if !reflect.DeepEqual(records, want) {
				t.Fatalf("decoded records: %#v", records)
			}
		})
	}
	for name, wire := range map[string][]byte{
		"unknown":         {0x81, 0xa1, 'x', 0},
		"empty map":       {0x80},
		"extra tag":       {0x82, 0xa8, 'l', 'e', 'f', 't', '_', 'o', 'u', 't', 0, 0xa1, 'x', 0},
		"tuple missing":   {0x81, 0xa6, 'o', 'u', 't', 'p', 'u', 't', 0x91, 0xa6, 's', 't', 'd', 'o', 'u', 't'},
		"tuple extra":     {0x81, 0xa6, 'o', 'u', 't', 'p', 'u', 't', 0x93, 0xa6, 's', 't', 'd', 'o', 'u', 't', 0xc4, 0, 0},
		"scalar as array": {0x81, 0xa8, 'l', 'e', 'f', 't', '_', 'o', 'u', 't', 0x91, 0},
		"negative":        {0x81, 0xa8, 'l', 'e', 'f', 't', '_', 'o', 'u', 't', 0xff},
		"trailing":        {0x81, 0xa8, 'l', 'e', 'f', 't', '_', 'o', 'u', 't', 0, 0},
		"text not bin":    {0x81, 0xa6, 'o', 'u', 't', 'p', 'u', 't', 0x92, 0xa6, 's', 't', 'd', 'o', 'u', 't', 0xa0},
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := kept.DecodeRecordMsgpack(wire); err == nil {
				t.Fatal("accepted invalid kept record")
			}
		})
	}
}
