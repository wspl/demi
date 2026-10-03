package hostaccess

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// Dense tables cover whole, suffix, open, overflowing, unsatisfiable and invalid Range headers.
// Cost: no IO, processes, or wall-clock waits.
func TestRangeAnswers(t *testing.T) {
	tests := []struct {
		header        *string
		size          uint64
		status        int
		start, length uint64
	}{
		{nil, 100, 200, 0, 100},
		{new("bytes=10-19"), 100, 206, 10, 10},
		{new("bytes=90-"), 100, 206, 90, 10},
		{new("bytes=-5"), 100, 206, 95, 5},
		{new("bytes=-500"), 100, 206, 0, 100},
		{new("bytes=50-5000"), 100, 206, 50, 50},
		{new("bytes=100-"), 100, 416, 0, 0},
		{new("bytes=-0"), 100, 416, 0, 0},
		{new("bytes=0-"), 0, 416, 0, 0},
		{new("bytes=-5"), 0, 416, 0, 0},
		{new("bytes=99999999999999999999-"), 100, 416, 0, 0},
		{new("bytes=10-99999999999999999999"), 100, 206, 10, 90},
		{new("bytes=-99999999999999999999"), 100, 206, 0, 100},
	}
	for _, invalid := range []string{"bytes=0-1,5-6", "bytes=9-3", "items=0-1", "bytes=-", "bytes=a-b", "bytes=٣-"} {
		tests = append(tests, struct {
			header        *string
			size          uint64
			status        int
			start, length uint64
		}{new(invalid), 100, 200, 0, 100})
	}
	for i, tt := range tests {
		t.Run(fmt.Sprint(i), func(t *testing.T) {
			answer := RangeOf(tt.header, tt.size)
			if answer.Status() != tt.status {
				t.Fatalf("status = %d, want %d", answer.Status(), tt.status)
			}
			got, ok := answer.Range()
			if ok != (tt.status != 416) {
				t.Fatalf("satisfiable = %v", ok)
			}
			if ok {
				if diff := cmp.Diff(host.ByteRange{Offset: tt.start, Length: new(tt.length)}, got); diff != "" {
					t.Fatal(diff)
				}
			}
			data := make([]byte, tt.size)
			for i := range data {
				data[i] = byte(i)
			}
			var part []byte
			if tt.status != 416 {
				part = data[tt.start : tt.start+tt.length]
			}
			if diff := cmp.Diff(part, answer.PartOf(data)); diff != "" {
				t.Fatal(diff)
			}
		})
	}
	headers := RangeOf(new("bytes=10-19"), 100).Headers()
	if headers.Get("Content-Range") != "bytes 10-19/100" || headers.Get("Content-Length") != "10" ||
		headers.Get("Accept-Ranges") != "bytes" {
		t.Fatal(headers)
	}
	if RangeOf(new("bytes=100-"), 100).Headers().Get("Content-Range") != "bytes */100" {
		t.Fatal("refused range headers")
	}
	if RangeOf(nil, 100).Headers().Get("Content-Length") != "100" {
		t.Fatal("whole range length")
	}
}

func TestFileVersionsAndWeakConditions(t *testing.T) {
	stamp, err := core.TimestampFromMillisecond(1790000000123)
	if err != nil {
		t.Fatal(err)
	}
	version := FileVersion(host.FileStat{Kind: host.File, Mode: 0o644, Size: 300000, Modified: stamp})
	if version != `W/"493e0-1a0c4506c7b"` {
		t.Fatal(version)
	}
	for _, condition := range []string{version, `"493e0-1a0c4506c7b"`, `"other", W/"493e0-1a0c4506c7b"`, " * "} {
		if !notModified(&condition, version) {
			t.Fatalf("condition %q did not match", condition)
		}
	}
	if notModified(new(`W/"493e0-0"`), version) || notModified(nil, version) {
		t.Fatal("unexpected match")
	}
	negative, err := core.TimestampFromMillisecond(-15)
	if err != nil {
		t.Fatal(err)
	}
	if got := FileVersion(host.FileStat{Size: 16, Modified: negative}); got != `W/"10--f"` {
		t.Fatal(got)
	}
}

// Closing revokes and drains every transfer; a cancelled close drops only its own hold,
// and admission reopens after the last hold.
// Synctest observes blocked work without wall time or scheduler guesses.
func TestTransferClosingDrainsAndReopensAfterLastHold(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		set := NewTransferSet(&sync.Mutex{})
		first, err := set.Open()
		if err != nil {
			t.Fatal(err)
		}
		second, err := set.Open()
		if err != nil {
			t.Fatal(err)
		}
		defer first.Release()
		defer second.Release()
		finished := make(chan *TransfersClosed, 1)
		go func() {
			hold, err := set.Close(t.Context())
			if err != nil {
				t.Error(err)
			}
			finished <- hold
		}()
		synctest.Wait()
		if first.Context().Err() == nil || second.Context().Err() == nil {
			t.Fatal("closing did not revoke both")
		}
		if _, err := set.Open(); err == nil {
			t.Fatal("closing admitted a transfer")
		}
		select {
		case <-finished:
			t.Fatal("close returned before release")
		default:
		}
		first.Release()
		synctest.Wait()
		select {
		case <-finished:
			t.Fatal("close missed second transfer")
		default:
		}
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		if _, err := set.Close(ctx); !errors.Is(err, context.Canceled) {
			t.Fatalf("cancelled close: %v", err)
		}
		second.Release()
		closed := <-finished
		defer closed.Release()
		other, err := set.Close(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		closed.Release()
		if _, err := set.Open(); err == nil {
			t.Fatal("first close reopened second hold")
		}
		other.Release()
		reopened, err := set.Open()
		if err != nil {
			t.Fatal(err)
		}
		defer reopened.Release()
		if reopened.Context().Err() != nil || !set.AnyOpen() {
			t.Fatal("new generation already ended")
		}
	})
}
