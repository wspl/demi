package commandservice_test

import (
	"bytes"
	"io"
	"sync"
	"testing"

	cs "github.com/wspl/demi/go/commandservice"
)

type loadHandler struct {
	started  chan string
	finished chan string
	release  chan struct{}
}

func (h *loadHandler) Operations() []string { return []string{"hold", "flood", "ping"} }

func (h *loadHandler) Invoke(c *cs.Call) (cs.Completion, error) {
	op := c.Invocation.Operation
	if op != "flood" {
		h.started <- op
	}
	defer func() { h.finished <- op }()
	switch op {
	case "hold":
		select {
		case <-h.release:
		case <-c.Context().Done():
			return cs.Completion{}, c.Context().Err()
		}
	case "flood":
		b := make([]byte, cs.MaxRecordBytes-36)
		for range 6 {
			if _, err := c.Stdout.Write(b); err != nil {
				return cs.Completion{}, err
			}
		}
		h.started <- op
		for {
			if _, err := c.Stdout.Write(b); err != nil {
				return cs.Completion{}, err
			}
		}
	}
	return cs.Completion{}, nil
}

// Budget 20 seconds. 160 held invocations exceed the default HTTP/2 limit;
// 80 unread streams exceed a 4 MiB connection window. No sleeps coordinate work.
func TestIndependentStreams(t *testing.T) {
	h := &loadHandler{
		started:  make(chan string, 512),
		finished: make(chan string, 512),
		release:  make(chan struct{}),
	}
	c, ctx := clientFor(t, h)
	var held []*cs.Stream
	for range 160 {
		s, err := c.Invoke(ctx, invocation("hold"))
		if err != nil {
			t.Fatal(err)
		}
		held = append(held, s)
	}
	for range held {
		select {
		case <-h.started:
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	close(h.release)
	for _, s := range held {
		if _, err := cs.Exchange(ctx, s, bytes.NewReader(nil), io.Discard, io.Discard); err != nil {
			t.Fatal(err)
		}
	}
	for range held {
		<-h.finished
	}
	var stalled []*cs.Stream
	for range 80 {
		s, err := c.Invoke(ctx, invocation("flood"))
		if err != nil {
			t.Fatal(err)
		}
		stalled = append(stalled, s)
	}
	for range stalled {
		select {
		case <-h.started:
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	run(t, ctx, c, invocation("ping"), nil)
	<-h.started
	<-h.finished
	for _, s := range stalled {
		s.Cancel()
	}
	for range stalled {
		select {
		case <-h.finished:
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	// A burst abandoned at response headers must leave this same connection usable.
	var wg sync.WaitGroup
	for range 160 {
		wg.Go(func() {
			s, err := c.Invoke(ctx, invocation("flood"))
			if err != nil {
				t.Error(err)
				return
			}
			s.Cancel()
		})
	}
	wg.Wait()
	run(t, ctx, c, invocation("ping"), nil)
}

func TestEarlyCompletionInput(t *testing.T) {
	h := &loadHandler{started: make(chan string, 1), finished: make(chan string, 1)}
	c, ctx := clientFor(t, h)
	s, err := c.Invoke(ctx, invocation("ping"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err = cs.Exchange(ctx, s, bytes.NewReader(nil), io.Discard, io.Discard); err != nil {
		t.Fatal(err)
	}
	if err = s.Write([]byte("late")); err != nil {
		t.Fatal(err)
	}
	if err = s.End(); err != nil {
		t.Fatal(err)
	}
}

func TestDrainKeepsRunningCall(t *testing.T) {
	h := &loadHandler{started: make(chan string, 1), finished: make(chan string, 1), release: make(chan struct{})}
	c, ctx := clientFor(t, h)
	s, err := c.Invoke(ctx, invocation("hold"))
	if err != nil {
		t.Fatal(err)
	}
	select {
	case <-h.started:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	if err = c.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	close(h.release)
	completion, err := cs.Exchange(ctx, s, bytes.NewReader(nil), io.Discard, io.Discard)
	if err != nil || completion.ExitCode != 0 {
		t.Fatal(completion, err)
	}
}
