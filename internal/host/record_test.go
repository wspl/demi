package host_test

import (
	"math"
	"reflect"
	"strings"
	"testing"
	"testing/synctest"
	"time"
	"unicode/utf8"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

func newRecord() *host.CommandRecord { return host.NewCommandRecord("shell", "command", "call") }

// The record tests use virtual time only; the complete suite costs less than one second.
func TestByteViewsAndRepeatedUnfinishedLines(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := newRecord()
		r.AppendOutput("stdout", "héllo ")
		r.AppendOutput("stderr", "warn\n")
		r.AppendOutput("stdout", "wörld\n")
		time.Sleep(250 * time.Millisecond) // Virtual elapsed time is the behavior under test.
		first := r.Status(2, nil)
		if first.Stdout.Delta != "h" || first.Stdout.Offset != 1 || !first.Stdout.Truncated || first.Stderr.Delta != "wa" || first.Output.Text != "h" || first.Output.Offset != 0 || first.RunningMs != 250 || first.State.Phase != host.Running || first.State.Hint != nil {
			t.Fatalf("first %+v", first)
		}
		hint := "Working"
		rest := r.Status(0, &hint)
		want := []core.OutputChunk{{Stream: "stdout", Text: "héllo "}, {Stream: "stderr", Text: "warn\n"}, {Stream: "stdout", Text: "wörld\n"}}
		if rest.Stdout.Delta != "éllo wörld\n" || rest.Stdout.Bytes != uint64(len("héllo wörld\n")) || rest.Stdout.Truncated || rest.Stdout.Tail != "héllo wörld\n" || rest.Stderr.Delta != "rn\n" || !reflect.DeepEqual(rest.Output.Chunks, want) || rest.State.Phase != host.Running || rest.State.Hint == nil || *rest.State.Hint != hint {
			t.Fatalf("rest %+v", rest)
		}
		empty := r.Status(0, nil)
		if empty.Stdout.Delta != "" || len(empty.Output.Chunks) != 0 || empty.IdleMs != 250 {
			t.Fatalf("empty %+v", empty)
		}
		r.AppendOutput("stdout", "next\n")
		more := r.Status(0, nil)
		if more.Output.Text != "next\n" || more.Output.Line != 3 {
			t.Fatalf("more %+v", more.Output)
		}
	})
}
func TestEndGivesWholeOutputOnceWithSeen(t *testing.T) {
	r := newRecord()
	r.AppendOutput("stdout", "head\n")
	if r.Status(0, nil).Output.Text != "head\n" {
		t.Fatal("head missing")
	}
	whole := &host.WholeOutput{Records: []host.OutputRecord{{Stream: "stdout", Bytes: []byte("head\nmore\n")}, {Stream: "stderr", Bytes: []byte("oops\n")}}}
	binary := &host.BinaryOutput{Bytes: []byte("\x89PNG"), Info: core.BinaryStdout{TotalBytes: 4, LimitBytes: 16}}
	if !r.Settle(host.Ending{Phase: host.Exited, ExitCode: 3}, whole, binary, "") {
		t.Fatal("first end not reported")
	}
	exited := r.Status(0, nil)
	if exited.Whole == nil || !reflect.DeepEqual(exited.Whole.Output, whole) || exited.Whole.Seen != (host.Seen{Stdout: 5}) || exited.State.Phase != host.Exited || exited.State.ExitCode != 3 || !reflect.DeepEqual(exited.State.BinaryStdout, binary) {
		t.Fatalf("exit %+v", exited)
	}
	again := r.Status(0, nil)
	if again.Whole == nil || again.Whole.Seen != (host.Seen{Stdout: math.MaxUint64, Stderr: math.MaxUint64}) {
		t.Fatalf("again %+v", again)
	}
}
func TestAbortRetainsUnsettledViews(t *testing.T) {
	r := newRecord()
	r.AppendOutput("stderr", "partial")
	if !r.Settle(host.Ending{Phase: host.Aborted}, &host.WholeOutput{}, nil, "") || r.Status(0, nil).State.Phase != host.Aborted {
		t.Fatal("settled abort")
	}
	r = newRecord()
	r.AppendOutput("stdout", strings.Repeat("x", host.TailChars)+"é")
	r.MarkAborted()
	status := r.Status(1, nil)
	if status.State.Phase != host.Aborted || status.Whole != nil || utf8.RuneCountInString(status.Stdout.Tail) != host.TailChars || !strings.HasSuffix(status.Stdout.Tail, "é") || utf8.RuneCountInString(status.Output.Tail) != host.TailChars {
		t.Fatalf("abort %+v", status)
	}
}
func TestPagesKeepNewestCharactersUntilEnd(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := newRecord()
		if !r.AppendOutput("stdout", "é") || r.AppendOutput("stdout", "") {
			t.Fatal("page change flags")
		}
		r.Status(0, nil)
		beyond := strings.Repeat("x", host.TailChars)
		if !r.AppendPageOutput(beyond) {
			t.Fatal("page-only output")
		}
		time.Sleep(40 * time.Millisecond)
		view := r.PageView()
		if view.Tail != beyond || view.Chars != 1+host.TailChars || view.State.Phase != host.Running || view.RunningMs != 40 || view.ToolUseID != "call" {
			t.Fatalf("view %+v", view)
		}
		if r.Status(0, nil).Output.Text != "é" {
			t.Fatal("page advanced model")
		}
		if !r.Settle(host.Ending{Phase: host.Exited, ExitCode: 2}, &host.WholeOutput{}, nil, "end\n") || r.AppendOutput("stderr", "late") || r.MarkAborted() {
			t.Fatal("end changed twice")
		}
		view = r.PageView()
		if view.State.Phase != host.Exited || view.State.ExitCode != 2 || !strings.HasSuffix(view.Tail, "xend\n") || utf8.RuneCountInString(view.Tail) != host.TailChars || view.Chars != 1+host.TailChars+4 {
			t.Fatalf("end view %+v", view)
		}
	})
}
func TestGrowthAndSeenUseMergedLineBoundary(t *testing.T) {
	r := newRecord()
	r.AppendOutput("stdout", "line\npart")
	r.AppendOutput("stderr", "ial")
	r.Grew("stdout", 100)
	r.Grew("stdout", 50)
	r.SetNewest("stdout", 90, 81, "new")
	status := r.Status(0, nil)
	if status.Unreceived != 91 || status.Stdout.Bytes != 100 || len(status.Newest) != 1 || status.Output.Offset != 5 {
		t.Fatalf("growth %+v", status)
	}
	r.Settle(host.Ending{Phase: host.Exited}, &host.WholeOutput{}, nil, "")
	ended := r.Status(0, nil)
	if ended.Whole.Seen != (host.Seen{Stdout: 5}) || ended.Unreceived != 0 || len(ended.Newest) != 0 {
		t.Fatalf("end %+v", ended)
	}
	r = newRecord()
	r.AppendOutput("stdout", "é")
	if got := r.Status(1, nil); got.Stdout.Delta != "é" || got.Stdout.Offset != 2 {
		t.Fatalf("tiny budget stalls: %+v", got)
	}
}
