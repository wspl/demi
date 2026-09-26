// Package contracttest runs a contract package's recorded corpus: values the
// TypeScript codecs encoded and Zod judged (scripts/record-go-contracts.ts).
// Go must accept exactly what Zod accepted and encode it to the same bytes.
package contracttest

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"os"
	"testing"
)

// Codec decodes one encoded value and encodes the result again.
type Codec func(input []byte) ([]byte, error)

// Roots maps a root type and a codec name ("json", "msgpack") to its codec.
type Roots map[string]map[string]Codec

// RoundTrip is the codec of a root: its decoder, then its encoder.
func RoundTrip[T any](decode func([]byte) (T, error), encode func(T) ([]byte, error)) Codec {
	return func(input []byte) ([]byte, error) {
		value, err := decode(input)
		if err != nil {
			return nil, err
		}
		return encode(value)
	}
}

// Case is one recorded value. Output is what TypeScript encodes after Zod
// parses Input, or null when Zod rejects Input. A JSON case holds text; a
// MessagePack case holds base64.
type Case struct {
	Root   string  `json:"root"`
	Codec  string  `json:"codec"`
	Name   string  `json:"name"`
	Input  string  `json:"input"`
	Output *string `json:"output"`
}

// Corpus is a package's recorded cases.
type Corpus struct {
	Cases []Case `json:"cases"`
}

// Run checks every case of the corpus file against the roots, and that the
// corpus covers every root and codec.
func Run(t *testing.T, path string, roots Roots) {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read corpus: %v", err)
	}
	var corpus Corpus
	if err := json.Unmarshal(data, &corpus); err != nil {
		t.Fatalf("parse corpus: %v", err)
	}
	covered := map[string]bool{}
	for _, entry := range corpus.Cases {
		codec := roots[entry.Root][entry.Codec]
		if codec == nil {
			t.Errorf("%s %s: no Go codec for the recorded root", entry.Root, entry.Codec)
			continue
		}
		covered[entry.Root+" "+entry.Codec] = true
		runCase(t, entry, codec)
	}
	for root, codecs := range roots {
		for name := range codecs {
			if !covered[root+" "+name] {
				t.Errorf("%s %s: the corpus records no case", root, name)
			}
		}
	}
}

func runCase(t *testing.T, entry Case, codec Codec) {
	t.Helper()
	label := entry.Root + " " + entry.Codec + " " + entry.Name
	input, err := caseBytes(entry.Codec, entry.Input)
	if err != nil {
		t.Errorf("%s: input: %v", label, err)
		return
	}
	got, err := codec(input)
	if entry.Output == nil {
		if err == nil {
			t.Errorf("%s: Zod rejects %q; Go accepted it as %q", label, entry.Input, got)
		}
		return
	}
	want, err2 := caseBytes(entry.Codec, *entry.Output)
	if err2 != nil {
		t.Errorf("%s: output: %v", label, err2)
		return
	}
	if err != nil {
		t.Errorf("%s: Zod accepts %q; Go rejected it: %v", label, entry.Input, err)
		return
	}
	if !bytes.Equal(got, want) {
		t.Errorf("%s: encoded\n got %q\nwant %q", label, got, want)
		return
	}
	again, err := codec(want)
	if err != nil || !bytes.Equal(again, want) {
		t.Errorf("%s: the canonical encoding does not round-trip: %q, %v", label, again, err)
	}
}

func caseBytes(codec, text string) ([]byte, error) {
	if codec == "msgpack" {
		return base64.StdEncoding.DecodeString(text)
	}
	return []byte(text), nil
}
