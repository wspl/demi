package commandservice_test

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"sync"
	"testing"
	"time"

	cs "github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

type lifecycleHandler struct {
	entered, stopped, closed chan struct{}
	once                     sync.Once
}

func (h *lifecycleHandler) Operations() []string { return []string{"ping"} }

func (h *lifecycleHandler) Invoke(*cs.Call) (cs.Completion, error) { return cs.Completion{}, nil }

func (h *lifecycleHandler) Conversation(c *cs.ConversationCall) (cs.Completion, error) {
	if c.Request.Conversation == "fail" {
		return cs.Completion{}, errors.New("cleanup failed")
	}
	close(h.entered)
	<-c.Context().Done()
	close(h.stopped)
	return cs.Completion{}, c.Context().Err()
}

func (h *lifecycleHandler) Close(context.Context) error {
	h.once.Do(func() { close(h.closed) })
	return nil
}

// Lifecycle scenarios have a 10 second budget and synchronize on handler events.
func TestConversationLifecycle(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	h := &lifecycleHandler{entered: make(chan struct{}), stopped: make(chan struct{}), closed: make(chan struct{})}
	a, b := net.Pipe()
	t.Cleanup(func() {
		cancel()
		// Either side may already be closed by service teardown.
		_ = a.Close()
		_ = b.Close()
	})
	done := make(chan error, 1)
	go func() { done <- cs.Serve(ctx, a, h) }()
	c, err := cs.Connect(ctx, b)
	if err != nil {
		t.Fatal(err)
	}
	// Teardown may already have closed the peer; closing only releases transport resources.
	defer c.Close()
	s, err := c.Conversation(ctx, cs.ConversationRequest{Operation: "release", Conversation: "wait"})
	if err != nil {
		t.Fatal(err)
	}
	select {
	case <-h.entered:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	s.Cancel()
	select {
	case <-h.stopped:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	run(t, ctx, c, invocation("ping"), nil)
	s, err = c.Conversation(ctx, cs.ConversationRequest{Operation: "release", Conversation: "fail"})
	if err != nil {
		t.Fatal(err)
	}
	r, err := s.Next()
	if err == nil && r.Kind == cs.Completed && r.Completion.ExitCode == 0 {
		t.Fatal(r, err)
	}
	select {
	case err = <-done:
		if !errors.Is(err, cs.ErrConversationCleanup) {
			t.Fatal(err)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	select {
	case <-h.closed:
	default:
		t.Fatal("Serve returned before Close")
	}
}

func TestNumbersConversationScope(t *testing.T) {
	c, ctx := clientFor(t, servicetest.NewFixture())
	waiting, err := c.Invoke(ctx, invocation("number"))
	if err != nil {
		t.Fatal(err)
	}
	numbers, err := c.Numbers(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = c.Numbers(ctx); err == nil {
		t.Fatal("second numbers stream accepted")
	} else {
		var rejected *cs.RejectedError
		if !errors.As(err, &rejected) || rejected.Status != http.StatusConflict {
			t.Fatal(err)
		}
	}
	answered := make(chan error, 1)
	var mu sync.Mutex
	counts := make(map[string]uint64)
	go func() {
		answered <- numbers.Answer(ctx, func(_ context.Context, r cs.NumbersRequest) (uint64, error) {
			mu.Lock()
			defer mu.Unlock()
			first := counts[r.Conversation] + 1
			counts[r.Conversation] += uint64(r.Count)
			return first, nil
		})
	}()
	var out bytes.Buffer
	if _, err = cs.Exchange(ctx, waiting, bytes.NewReader(nil), &out, io.Discard); err != nil || out.String() != `{"first":1}` {
		t.Fatal(out.String(), err)
	}
	v := invocation("number")
	v.Context.Conversation = "c2"
	result, _, _ := run(t, ctx, c, v, nil)
	if string(result) != `{"first":1}` {
		t.Fatal(string(result))
	}
	if err = c.Shutdown(ctx); err != nil {
		t.Fatal(err)
	}
	if err = <-answered; err != nil {
		t.Fatal(err)
	}
}

type stuckHandler struct{ entered, release, stopped chan struct{} }

func (h *stuckHandler) Operations() []string { return []string{"stuck"} }

func (h *stuckHandler) Invoke(*cs.Call) (cs.Completion, error) {
	close(h.entered)
	<-h.release
	close(h.stopped)
	return cs.Completion{}, nil
}

// This fault scenario costs the protocol's five-second grace, budget eight seconds.
// It waits for service retirement, not a test sleep; release joins the planted fault.
func TestCancellationDeadlineIsNotWaitedTwice(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
	defer cancel()
	h := &stuckHandler{entered: make(chan struct{}), release: make(chan struct{}), stopped: make(chan struct{})}
	a, b := net.Pipe()
	t.Cleanup(func() {
		cancel()
		// Either side may already be closed by service teardown.
		_ = a.Close()
		_ = b.Close()
	})
	done := make(chan error, 1)
	go func() { done <- cs.Serve(ctx, a, h) }()
	c, err := cs.Connect(ctx, b)
	if err != nil {
		t.Fatal(err)
	}
	// Teardown may already have closed the peer; closing only releases transport resources.
	defer c.Close()
	defer func() {
		close(h.release)
		select {
		case <-h.entered:
			<-h.stopped
		default:
		}
	}()
	s, err := c.Invoke(ctx, invocation("stuck"))
	if err != nil {
		t.Fatal(err)
	}
	<-h.entered
	s.Cancel()
	select {
	case err = <-done:
		if !errors.Is(err, cs.ErrCancellationDeadline) {
			t.Fatal(err)
		}
	case <-ctx.Done():
		t.Fatal("service waited beyond the cancellation grace")
	}
}
