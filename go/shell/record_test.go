package shell_test

import (
	"math"
	"reflect"
	"slices"
	"strings"
	"testing"
	"testing/synctest"
	"time"
	"unicode/utf8"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/shell"
)

func record(t *testing.T) *shell.CommandRecord {
	t.Helper()
	sid, err := core.ParseShellID("shell")
	if err != nil {
		t.Fatal(err)
	}
	cid, err := core.ParseCommandID("command")
	if err != nil {
		t.Fatal(err)
	}
	return shell.NewCommandRecord(sid, cid, "call")
}

// These scenarios use synthetic time and a few KiB of output; no external work.
func TestModelCursorCommitsOnlyWholeLines(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := record(t)
		r.AppendOutput(core.StreamKindStdout, "héllo ")
		r.AppendOutput(core.StreamKindStderr, "warn\n")
		r.AppendOutput(core.StreamKindStdout, "wörld\npartial")
		time.Sleep(250 * time.Millisecond)
		first := r.Status(2, nil)
		if first.Stdout.Delta != "h" || first.Stdout.Offset != 1 || !first.Stdout.Truncated || first.Stderr.Delta != "wa" || first.Output.Text != "h" || first.Output.Offset != 0 || first.RunningMs != 250 {
			t.Fatalf("first bounded view: %+v", first)
		}
		next := r.Status(0, new("Working"))
		if next.Stdout.Delta != "éllo wörld\npartial" || next.Stderr.Delta != "rn\n" || next.Output.Text != "héllo warn\nwörld\npartial" || next.Output.Offset != 19 || next.IdleMs != 250 {
			t.Fatalf("next view: %+v", next)
		}
		repeated := r.Status(0, nil)
		if repeated.Output.Text != "partial" || repeated.Output.Line != 3 || repeated.Stdout.Delta != "" {
			t.Fatalf("unfinished line: %+v", repeated)
		}
		whole := shell.NewWholeOutput([]shell.OutputRecord{
			shell.OutputRead{Stream: core.StreamKindStdout, Bytes: []byte("héllo wörld\npartial")},
			shell.OutputRead{Stream: core.StreamKindStderr, Bytes: []byte("warn\n")},
		}, nil)
		r.Settle(shell.Ending{ExitCode: 3}, whole, nil, "")
		ended := r.Status(0, nil)
		if ended.Whole.Seen != (shell.Seen{Stdout: 14, Stderr: 5}) || ended.Output.Text != "partial" || ended.Output.Offset != 26 || ended.State.ExitCode != 3 {
			t.Fatalf("end repeats incomplete line and projects committed cursor: %+v, seen %+v", ended, ended.Whole.Seen)
		}
		if again := r.Status(0, nil); again.Whole.Seen != (shell.Seen{Stdout: math.MaxUint64, Stderr: math.MaxUint64}) || again.Output.Text != "" {
			t.Fatalf("second final view: %+v", again)
		}
	})
}
func TestPagePreviewAndHostGrowthAreIndependentOfModelReads(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := record(t)
		r.AppendOutput(core.StreamKindStdout, "é")
		if got := r.Status(1, nil).Stdout.Delta; got != "é" {
			t.Fatalf("a tiny budget must advance one character: %q", got)
		}
		r.AppendPageOutput(strings.Repeat("x", 4096))
		time.Sleep(40 * time.Millisecond)
		r.Grew(core.StreamKindStdout, 9000)
		r.Grew(core.StreamKindStdout, 8000)
		status := r.Status(0, nil)
		page := r.PageView()
		if status.Unreceived != 8998 || status.IdleMs != 0 || status.Output.Text != "é" || page.Chars != 4097 || page.Tail != strings.Repeat("x", 4096) || page.RunningMs != 40 {
			t.Fatalf("growth or independent preview: status=%+v page=%+v", status, page)
		}
		r.Settle(shell.Ending{ExitCode: 2}, shell.NewWholeOutput(nil, nil), nil, "end\n")
		if r.AppendOutput(core.StreamKindStderr, "late") || r.MarkAborted() {
			t.Fatal("ended page changed")
		}
		page = r.PageView()
		if page.State.Phase != shell.CommandExited || page.State.ExitCode != 2 || page.Chars != 4101 || utf8.RuneCountInString(page.Tail) != 4096 || !strings.HasSuffix(page.Tail, "xend\n") {
			t.Fatalf("final page: %+v", page)
		}
		stopped := record(t)
		stopped.AppendOutput(core.StreamKindStderr, "partial")
		if !stopped.MarkAborted() || stopped.MarkAborted() {
			t.Fatal("abort transition")
		}
		if status := stopped.Status(0, nil); status.State.Phase != shell.CommandAborted || status.Stderr.Delta != "partial" || status.Whole != nil {
			t.Fatalf("stopped streams: %+v", status)
		}
	})
}
func TestWholeOutputKeepsGapFragmentsAndStreamOrder(t *testing.T) {
	whole := shell.NewWholeOutput([]shell.OutputRecord{
		shell.OutputRead{Stream: core.StreamKindStdout, Bytes: []byte("one\nleft")}, shell.OutputLeftOut(100),
		shell.OutputRead{Stream: core.StreamKindStderr, Bytes: []byte("right\nlast")},
	}, &shell.Missing{Bytes: 7, Reason: "lost with the Host's connection"})
	text := whole.Text(shell.Streams{}, nil, shell.Seen{Stdout: 8})
	pieces := slices.Collect(text.Forward(1))
	want := []shell.Piece{{Number: 1, Offset: 0, Bytes: []byte("one")}, {Number: 2, Offset: 4, Bytes: []byte("left")}, {Note: "[... 100 bytes left out ...]"}, {Number: 2, Offset: 8, Bytes: []byte("right")}, {Number: 3, Offset: 14, Bytes: []byte("last")}, {Note: "[... 7 bytes lost with the Host's connection ...]"}}
	if !reflect.DeepEqual(pieces, want) {
		t.Fatalf("forward: %#v", pieces)
	}
	back := slices.Collect(text.Backward())
	slices.Reverse(want)
	if !reflect.DeepEqual(back, want) {
		t.Fatalf("backward: %#v", back)
	}
	if string(text.Bytes()) != "one\nleftright\nlast" || *text.Unseen() != 8 || *text.UnseenLine() != 2 || text.LastLine() != 3 || text.Column(18) != 4 {
		t.Fatalf("line and seen views: %+v", text)
	}
	chunks := text.Chunks(4)
	if len(chunks) != 4 || chunks[0].Text != "left" || chunks[1].Text != "\n[... 100 bytes left out ...]\n" || chunks[1].Stream != core.StreamKindStderr {
		t.Fatalf("terminal chunks: %+v", chunks)
	}
}
func TestBinaryAndCutUTF8Output(t *testing.T) {
	for _, tc := range []struct {
		name    string
		records []shell.OutputRecord
		binary  bool
	}{
		{"split read", []shell.OutputRecord{shell.OutputRead{Stream: core.StreamKindStdout, Bytes: []byte{0xc3}}, shell.OutputRead{Stream: core.StreamKindStdout, Bytes: []byte{0xa9}}}, false},
		{"gap edges", []shell.OutputRecord{shell.OutputRead{Stream: core.StreamKindStdout, Bytes: []byte{0xe2, 0x82}}, shell.OutputLeftOut(5), shell.OutputRead{Stream: core.StreamKindStdout, Bytes: []byte{0xac, 'x'}}}, false},
		{"binary", []shell.OutputRecord{shell.OutputRead{Stream: core.StreamKindStdout, Bytes: []byte{0x89, 'P', 'N', 'G'}}}, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			whole := shell.NewWholeOutput(tc.records, nil)
			length := whole.BinaryStdoutLength()
			if (length != nil) != tc.binary {
				t.Fatalf("binary length: %v", length)
			}
			if tc.binary {
				if binary := whole.BinaryStdout(4, 4); binary.Info.Truncated || len(binary.Bytes) != 4 {
					t.Fatalf("whole binary: %+v", binary)
				}
				if binary := whole.BinaryStdout(4, 3); !binary.Info.Truncated || len(binary.Bytes) != 0 {
					t.Fatalf("limited binary: %+v", binary)
				}
				if got := whole.Text(shell.Streams{}, length, shell.Seen{}).Display(); got != "<binary stdout: 4 bytes>\n" {
					t.Fatalf("binary display: %q", got)
				}
			}
		})
	}
	if got := (shell.Piece{Bytes: []byte{0xe2, 0x82}}).Text(); got != "�" {
		t.Fatalf("Rust maximal-subpart replacement: %q", got)
	}
}
