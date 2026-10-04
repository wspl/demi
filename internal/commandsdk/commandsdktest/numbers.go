package commandsdktest

import (
	"context"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
)

type counters struct {
	mu   sync.Mutex
	next map[string]uint64
}

func (c *counters) take(q commandproto.NumbersRequest) uint64 {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.next == nil {
		c.next = map[string]uint64{}
	}
	key := q.Conversation + ":" + string(q.Sequence)
	first := c.next[key]
	if first == 0 {
		first = 1
	}
	c.next[key] = first + uint64(q.Count)
	return first
}

// CountingNumbers supplies per-conversation sequences from 1, with test-owned cleanup.
func CountingNumbers(t testing.TB) *commandsdk.Numbers {
	t.Helper()
	n, requests := commandsdk.NumbersChannel()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		defer close(done)
		var c counters
		for {
			select {
			case <-ctx.Done():
				return
			case p := <-requests:
				p.Reply(c.take(p.Request), nil)
			}
		}
	}()
	t.Cleanup(func() {
		cancel()
		n.Close()
		<-done
	})
	return n
}

// AnswerNumbers opens and answers a service stream until shutdown or context cancellation.
func AnswerNumbers(ctx context.Context, client *commandsdk.Client) error {
	s, err := client.Numbers(ctx)
	if err != nil {
		return err
	}
	var c counters
	return s.AnswerNumbers(
		ctx,
		func(_ context.Context, q commandproto.NumbersRequest) (uint64, error) { return c.take(q), nil },
	)
}
