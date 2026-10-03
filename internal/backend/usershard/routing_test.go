package usershard

import (
	"context"
	"errors"
	"testing"
	"testing/synctest"
)

// These scenarios use channels only, with no external services or wall-clock waits.
func TestCancelledRequesterStillFinishesAdmittedCall(t *testing.T) {
	s := &Shard{}
	ctx, cancel := context.WithCancel(t.Context())
	entered := make(chan struct{})
	completed := make(chan error, 1)
	observed := make(chan string, 1)
	go func() {
		_, err := shardCall(ctx, s, func(ctx context.Context) (struct{}, error) {
			close(entered)
			<-ctx.Done()
			observed <- "cancelled, then finished"
			return struct{}{}, ctx.Err()
		})
		completed <- err
	}()
	<-entered
	cancel()
	if err := <-completed; !errors.Is(err, context.Canceled) {
		t.Fatalf("call cancellation = %v", err)
	}
	s.calls.Wait()
	if got := <-observed; got != "cancelled, then finished" {
		t.Fatal(got)
	}
}

func TestPanickingCallDoesNotPoisonShard(t *testing.T) {
	s := &Shard{}
	_, err := shardCall(t.Context(), s, func(context.Context) (int, error) {
		panic("planted failure")
	})
	if !errors.Is(err, errShardFailed) {
		t.Fatalf("panic result = %v", err)
	}
	value, err := shardCall(t.Context(), s, func(context.Context) (int, error) { return 42, nil })
	if err != nil || value != 42 {
		t.Fatalf("subsequent call = %d, %v", value, err)
	}
}

func TestClosingWaitsForCallsAndRefusesNewOnes(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		s := &Shard{ctx: ctx, cancel: cancel}
		entered, release, done := make(chan struct{}), make(chan struct{}), make(chan struct{})
		go func() {
			defer close(done)
			_, err := shardCall(ctx, s, func(context.Context) (struct{}, error) {
				close(entered)
				<-release
				return struct{}{}, nil
			})
			if err != nil {
				t.Error(err)
			}
		}()
		<-entered
		drained := make(chan struct{})
		go func() {
			s.drainCalls()
			close(drained)
		}()
		<-s.Closed()
		synctest.Wait()
		select {
		case <-drained:
			t.Fatal("shutdown abandoned the admitted call")
		default:
		}
		_, err := shardCall(ctx, s, func(context.Context) (struct{}, error) {
			t.Error("new call ran during shutdown")
			return struct{}{}, nil
		})
		if !errors.Is(err, ErrClosing) {
			t.Fatalf("new call = %v", err)
		}
		close(release)
		<-done
		<-drained
		_, err = shardCall(ctx, s, func(context.Context) (struct{}, error) {
			t.Error("call ran after shutdown")
			return struct{}{}, nil
		})
		if !errors.Is(err, ErrClosing) {
			t.Fatalf("call after shutdown = %v", err)
		}
	})
}
