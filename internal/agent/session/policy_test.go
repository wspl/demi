package session

import (
	"context"
	"errors"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// These checks exercise session-owned policies without constructing the
// transcript dependency. Cost: no IO, no wall-clock waits, bounded iterations.
func TestRetryPolicyBoundaries(t *testing.T) {
	p := DefaultRetryPolicy()
	for _, tc := range []struct {
		attempt uint32
		code    provider.ErrorCode
		wait    *time.Duration
		retry   bool
	}{
		{1, provider.Overloaded, nil, true}, {3, provider.RateLimit, new(30 * time.Second), true}, {4, provider.Overloaded, nil, false}, {1, provider.RateLimit, new(31 * time.Second), false}, {1, provider.AuthExpired, nil, false}, {1, provider.ContextLengthExceeded, nil, false},
	} {
		got := p.retries(tc.attempt, provider.Failure{Code: &tc.code, RetryAfter: tc.wait})
		if got != tc.retry {
			t.Errorf("attempt %d code %s: %v", tc.attempt, tc.code, got)
		}
	}
	if p.retries(1, provider.Failure{}) {
		t.Fatal("unclassified failure retried")
	}
	for attempt := uint32(1); attempt <= 3; attempt++ {
		for range 50 {
			delay := p.delay(attempt, nil)
			if delay < 0 || delay >= time.Second*time.Duration(1<<(attempt-1)) {
				t.Fatalf("retry %d waits %s", attempt, delay)
			}
		}
	}
	if got := p.delay(1, new(1500*time.Millisecond)); got != 1500*time.Millisecond {
		t.Fatal(got)
	}
}
func TestCompactionThresholdCountsCacheAndLimits(t *testing.T) {
	c := DefaultCompactionConfig()
	if c.reached(1000, core.TokenUsage{InputTokens: 399, CacheReadTokens: 400}) {
		t.Fatal("compacted below threshold")
	}
	if !c.reached(1000, core.TokenUsage{InputTokens: 100, OutputTokens: 100, CacheReadTokens: 300, CacheWriteTokens: 300}) {
		t.Fatal("cache omitted")
	}
	limits := provider.RequestLimits{Images: new(uint32(5)), BodyBytes: new(uint64(1000))}
	for _, tc := range []struct {
		size transcript.RequestSize
		want bool
	}{{transcript.RequestSize{Images: 3, Bytes: 799}, false}, {transcript.RequestSize{Images: 4}, true}, {transcript.RequestSize{Bytes: 800}, true}} {
		if got := c.sizeReached(limits, tc.size); got != tc.want {
			t.Fatalf("%+v: %v", tc.size, got)
		}
	}
	c.ThresholdPercent = nil
	if c.sizeReached(limits, transcript.RequestSize{Images: 5, Bytes: 1000}) || c.reached(1000, core.TokenUsage{InputTokens: 1000}) {
		t.Fatal("disabled compaction triggered")
	}
}
func TestActionWaitCancellationDoesNotCancelResult(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		a := newAction()
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		if _, err := a.Wait(ctx); !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
		done := make(chan ActionEnd, 2)
		for range 2 {
			go func() {
				end, err := a.Wait(t.Context())
				if err != nil {
					t.Error(err)
				}
				done <- end
			}()
		}
		synctest.Wait()
		a.finish(Completed, nil)
		a.finish(Dropped, nil)
		for range 2 {
			if end := <-done; end != Completed {
				t.Fatal(end)
			}
		}
	})
}
func TestEditAcceptanceRemainsAfterWaitCancellation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		a := newAcceptance()
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		if _, err := a.Wait(ctx); !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
		receipt := store.EditReceipt{OperationID: "op", Digest: "digest", TurnID: "turn"}
		a.finish(receipt, nil)
		a.finish(store.EditReceipt{}, &EditError{Kind: EditStopped})
		got, err := a.Wait(t.Context())
		if err != nil || got != receipt {
			t.Fatalf("%+v, %v", got, err)
		}
	})
}
func TestKeptEditMediaMustBelongToTarget(t *testing.T) {
	target := []core.UserContentBlock{&core.UserAttachment{Path: "/a"}}
	got, err := resolveEdit([]EditContent{&KeptAttachment{Path: "/a"}}, target)
	if err != nil || len(got) != 1 || got[0] != target[0] {
		t.Fatalf("%+v, %v", got, err)
	}
	_, err = resolveEdit([]EditContent{&KeptAttachment{Path: "/b"}}, target)
	var refused *EditError
	if !errors.As(err, &refused) || refused.Kind != EditUnknownAttachment {
		t.Fatal(err)
	}
}
