package host_test

import (
	"fmt"
	"iter"
	"reflect"
	"slices"
	"testing"

	"github.com/wspl/demi/internal/host"
)

// These in-memory output scenarios cost less than one second and start no workers.
func pieceLines(pieces iter.Seq[host.Piece]) []string {
	var lines []string
	for p := range pieces {
		if p.Number == 0 {
			lines = append(lines, p.Note)
		} else {
			lines = append(lines, fmt.Sprintf("%d:%s", p.Number, p.Text()))
		}
	}
	return lines
}

func TestGapKeepsRawLineNumbering(t *testing.T) {
	gap := uint64(9)
	whole := host.WholeOutput{
		Records: []host.OutputRecord{
			{Stream: "stdout", Bytes: []byte("one\ntw")},
			{LeftOut: &gap},
			{Stream: "stderr", Bytes: []byte("o\nthree\n")},
		},
		Missing: &host.Missing{Bytes: 4, Reason: "lost with the Host's connection"},
	}
	text := whole.Text(host.Both, nil, host.Seen{})
	if string(text.Bytes()) != "one\ntwo\nthree\n" || text.LineCount() != 3 {
		t.Fatalf("raw: %q, count %d", text.Bytes(), text.LineCount())
	}
	forward := []string{
		"1:one",
		"2:tw",
		"[... 9 bytes left out ...]",
		"2:o",
		"3:three",
		"[... 4 bytes lost with the Host's connection ...]",
	}
	if got := pieceLines(text.Forward(1)); !reflect.DeepEqual(got, forward) {
		t.Fatalf("forward: %q", got)
	}
	if got := pieceLines(text.Forward(3)); got[0] != "3:three" {
		t.Fatalf("from 3: %q", got)
	}
	slices.Reverse(forward)
	if got := pieceLines(text.Backward()); !reflect.DeepEqual(got, forward) {
		t.Fatalf("backward: %q", got)
	}
}

func TestGapCutTextIsNotBinary(t *testing.T) {
	gap := uint64(3)
	cut := host.WholeOutput{
		Records: []host.OutputRecord{
			{Stream: "stdout", Bytes: []byte("ok \xc3")},
			{LeftOut: &gap},
			{Stream: "stdout", Bytes: []byte("\xbc!")},
		},
	}
	if got := cut.BinaryStdout(9, 100); got != nil {
		t.Fatalf("cut text classified binary: %+v", got)
	}
	binary := host.WholeOutput{Records: []host.OutputRecord{{Stream: "stdout", Bytes: []byte{0xff, 0}}}}
	got := binary.BinaryStdout(2, 100)
	if got == nil || got.Info.Truncated || !reflect.DeepEqual(got.Bytes, []byte{0xff, 0}) {
		t.Fatalf("binary: %+v", got)
	}
	for _, tc := range []struct {
		length uint64
		limit  int
	}{{3, 100}, {2, 1}} {
		if got := binary.BinaryStdout(tc.length, tc.limit); got == nil || !got.Info.Truncated || len(got.Bytes) != 0 {
			t.Fatalf("partial binary: %+v", got)
		}
	}
}

func TestOutputReadersPreserveStreamsSeenAndNotes(t *testing.T) {
	gap := uint64(7)
	whole := host.WholeOutput{
		Records: []host.OutputRecord{
			{Stream: "stdout", Bytes: []byte("aé\n")},
			{Stream: "stderr", Bytes: []byte("err")},
			{LeftOut: &gap},
			{Stream: "stdout", Bytes: []byte("end\n")},
		},
		Missing: &host.Missing{Bytes: 2, Reason: "missing"},
	}
	text := whole.Text(host.Both, nil, host.Seen{Stdout: 4, Stderr: 1})
	if at, ok := text.Unseen(); !ok || at != 5 {
		t.Fatalf("unseen %d %v", at, ok)
	}
	if line, ok := text.UnseenLine(); !ok || line != 2 {
		t.Fatalf("unseen line %d %v", line, ok)
	}
	if text.LineOffset(2) != 4 || text.LineOf(5) != 2 || text.Column(3) != 2 || !text.IsLineStart(4) {
		t.Fatal("line locations do not match raw bytes")
	}
	chunks := text.Chunks(4)
	if len(chunks) != 4 || chunks[0].Stream != "stderr" || chunks[0].Text != "err" ||
		chunks[1].Text != "\n[... 7 bytes left out ...]\n" ||
		chunks[2].Stream != "stdout" ||
		chunks[3].Text != "[... 2 bytes missing ...]\n" {
		t.Fatalf("chunks %+v", chunks)
	}
	stdout := whole.Text(host.OnlyStdout, nil, host.Seen{})
	if string(stdout.Bytes()) != "aé\nend\n" {
		t.Fatalf("stdout %q", stdout.Bytes())
	}
	length := uint64(23)
	binary := whole.Text(host.Both, &length, host.Seen{})
	if string(binary.Bytes()) != "<binary stdout: 23 bytes>\nerr" {
		t.Fatalf("binary description %q", binary.Bytes())
	}
	received := host.ReceivedOutput("hé\nnext", 9)
	if received.LastLine() != 10 || received.Column(3) != 2 || received.Display() != "hé\nnext\n" {
		t.Fatalf("received %+v", received)
	}
}

func TestOutputLossyUTF8MatchesSequenceBoundaries(t *testing.T) {
	for _, tc := range []struct{ raw, want string }{
		{
			"\xe1\x80x",
			"�x",
		},
		{
			"\xf0\x90\x80x",
			"�x",
		},
		{
			"\xc3",
			"�",
		},
		{
			"\xe0\x80x",
			"��x",
		},
		{
			"\xff\xff",
			"��",
		},
	} {
		output := host.WholeOutput{Records: []host.OutputRecord{{Stream: "stderr", Bytes: []byte(tc.raw)}}}
		text := output.Text(host.Both, nil, host.Seen{})
		got := text.Display()
		if got != tc.want+"\n" {
			t.Fatalf("%x: %q want %q", tc.raw, got, tc.want)
		}
	}
}
