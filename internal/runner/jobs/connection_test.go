package jobs

import (
	"context"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
)

func TestBackendAnswerDeadlineStartsAfterQueueAdmission(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		handle, requests := NewConnection(ctx, make(chan []byte, 1))
		for range cap(handle.requests) {
			handle.requests <- &CallRequest{}
		}
		returned := make(chan error, 1)
		go func() {
			_, err := handle.Reserve(ctx, "conversation", cmdproto.TabSequence, 1)
			returned <- err
		}()
		synctest.Wait()
		// Waiting for room in the request queue has no backend-answer deadline.
		time.Sleep(20 * time.Second)
		select {
		case <-returned:
			t.Fatal("queue wait incorrectly used answer timeout")
		default:
		}
		<-requests
		synctest.Wait()
		admitted := time.Now()
		err := <-returned
		failure, ok := err.(*cmdpkgs.RuntimeError)
		if !ok || failure.Kind != cmdpkgs.Deadline || time.Since(admitted) != 15*time.Second {
			t.Fatalf("deadline kind or timing differs: %T after %v", err, time.Since(admitted))
		}
	})
}
