package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/tools/contractgen/testdata/features"
	"github.com/wspl/demi/tools/contractgen/testdata/runner"
)

// Generated boundaries, including external unions and recursive objects, have a
// one-second local CPU budget and no child processes or clocks.
func TestGeneratedFeatures(t *testing.T) {
	good := `{"id":"id_ok","body":{"type":"text","text":"hello"},"data":"Zg==","label":null,"extra":null}`
	value, err := features.DecodeRequest([]byte(good))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := json.Marshal(value); err != nil {
		t.Fatal(err)
	}
	if _, err := features.ParseIdentifier("id_ok"); err != nil {
		t.Fatal(err)
	}
	if _, err := features.ParseIdentifier("bad"); err == nil {
		t.Fatal("invalid ID constructor succeeded")
	}
	for name, input := range map[string]string{
		"duplicate":                  strings.Replace(good, `"id":"id_ok"`, `"id":"id_ok","id":"id_ok"`, 1),
		"nested duplicate":           strings.Replace(good, `"text":"hello"`, `"text":"hello","text":"hello"`, 1),
		"utf8":                       strings.Replace(good, "hello", string([]byte{0xff}), 1),
		"surrogate":                  strings.Replace(good, "hello", `\ud800`, 1),
		"strict":                     strings.Replace(good, `"label":null`, `"unknown":0,"label":null`, 1),
		"base64":                     strings.Replace(good, "Zg==", "Zh==", 1),
		"timestamp-independent null": strings.Replace(good, `"text":"hello"`, `"text":null`, 1),
		"trailing":                   good + " true",
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := features.DecodeRequest([]byte(input)); err == nil {
				t.Fatal("accepted invalid input")
			}
		})
	}
	tree := `{"name":"id_root","children":[{"name":"id_child","children":[]}],"future":42}`
	if _, err := features.DecodeTree([]byte(tree)); err != nil {
		t.Fatal(err)
	}
	_, err = features.DecodeTree([]byte(strings.Replace(tree, "id_child", "invalid", 1)))
	var fieldError *contract.Error
	if !errors.As(err, &fieldError) || fieldError.Path != "children[0].name" {
		t.Fatalf("lost error path: %v", err)
	}
	if _, err := features.DecodeTree([]byte(strings.Replace(tree, `"future":42`, `"parent":null`, 1))); err == nil {
		t.Fatal("optional null accepted")
	}
}

func TestNamedInstantiation(t *testing.T) {
	if _, err := features.DecodeNames([]byte(`{"items":["id_ok"]}`)); err != nil {
		t.Fatal(err)
	}
	if _, err := features.DecodeNames([]byte(`{"items":[]}`)); err == nil {
		t.Fatal("generic field lost its bound")
	}
}

func TestFlattenedValue(t *testing.T) {
	input := []byte(`{"id":"id_ok","text":"hello"}`)
	value, err := features.DecodeFlat(input)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	if string(encoded) != string(input) {
		t.Fatalf("flattened fields changed: %s", encoded)
	}
	if _, err := features.DecodeFlat([]byte(`{"id":"bad","text":"hello"}`)); err == nil {
		t.Fatal("flattened ID was not validated")
	}
}

func TestCyclicValue(t *testing.T) {
	tree := features.Tree{Name: "id_root", Children: []features.Tree{}}
	tree.Parent = &tree
	if err := tree.Validate(); err == nil {
		t.Fatal("accepted a cyclic value")
	}
}

func TestTimestampExtensions(t *testing.T) {
	for name, data := range map[string][]byte{
		"wrong extension":         {0xd6, 13, 0, 0, 0, 0},
		"fractional milliseconds": {0xd7, 255, 0, 0, 0, 4, 0, 0, 0, 0},
		"nanosecond overflow":     {0xc7, 12, 255, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0},
		"trailing":                {0xd6, 255, 0, 0, 0, 0, 0},
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := runner.DecodeTimestampMsgpack(data); err == nil {
				t.Fatal("accepted invalid timestamp")
			}
		})
	}
}

// Sorting records and timestamp extensions fill gaps in library defaults.
func TestRecordEncoding(t *testing.T) {
	input := []byte{0x82, 0xa6, 'v', 'a', 'l', 'u', 'e', 's', 0x82, 0xa1, 'a', 0xc0, 0xa1, 'z', 0xa1, 'x', 0xa2, 'a', 't', 0xd6, 0xff, 0, 0, 0, 0}
	value, err := features.DecodeRecordsMsgpack(input)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := value.MarshalMsgpack()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(input, encoded) {
		t.Fatalf("record encoding differs: %x", encoded)
	}
}
