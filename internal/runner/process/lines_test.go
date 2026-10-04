package process_test

import (
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/runner/process"
)

func TestLineSplitterChunks(t *testing.T) {
	var splitter process.LineSplitter
	for _, tc := range []struct {
		chunk string
		want  []string
	}{
		{"first li", nil}, {"ne\r\nsecond\n\nthi", []string{"first line", "second"}}, {"rd", nil},
	} {
		if got := splitter.Push([]byte(tc.chunk)); !reflect.DeepEqual(got, tc.want) {
			t.Fatalf("Push(%q) = %q, want %q", tc.chunk, got, tc.want)
		}
	}
	if got, ok := splitter.Finish(); !ok || got != "third" {
		t.Fatalf("Finish = %q", got)
	}
	var long process.LineSplitter
	if got := long.Push(
		[]byte(strings.Repeat("y", process.LineBytes+3)),
	); !reflect.DeepEqual(
		got,
		[]string{strings.Repeat("y", process.LineBytes)},
	) {
		t.Fatalf("long lines = %q", got)
	}
	if got, ok := long.Finish(); !ok || got != "yyy" {
		t.Fatalf("tail = %q", got)
	}
	if got, ok := new(process.LineSplitter).Finish(); ok {
		t.Fatalf("empty = %q", got)
	}
}

func TestStreamTailsAndMalformedText(t *testing.T) {
	tail := process.NewTail(5)
	tail.Push([]byte("first"))
	tail.Push([]byte(" last"))
	if got := tail.Text(); got != " last" {
		t.Fatalf("tail = %q", got)
	}
	data := tail.Bytes()
	tail.Push([]byte("next"))
	if string(data) != " last" {
		t.Fatalf("transferred bytes changed: %q", data)
	}
	for _, tc := range []struct {
		data []byte
		want string
	}{
		{[]byte{0xff, 0xff}, "��"}, {[]byte{0xe2, 0x82}, "�"}, {[]byte{0xe2, 0x28, 0xa1}, "�(�"},
	} {
		tail := process.NewTail(10)
		tail.Push(tc.data)
		if got := tail.Text(); got != tc.want {
			t.Fatalf("%x = %q, want %q", tc.data, got, tc.want)
		}
	}
	zero := process.NewTail(0)
	zero.Push([]byte("discard"))
	if len(zero.Bytes()) != 0 {
		t.Fatal("zero-sized tail retained bytes")
	}
}
