package jobs

import (
	"context"
	"errors"
	"testing"

	"github.com/wspl/demi/internal/cmdsdk"
)

// blockedPipeInput represents finite command input the upload has requested but not received.
type blockedPipeInput struct{ reading chan struct{} }

func (s *blockedPipeInput) Next(ctx context.Context) ([]byte, error) {
	close(s.reading)
	<-ctx.Done()
	return nil, ctx.Err()
}

func TestEarlyPipeResponseInterruptsPendingCommandInput(t *testing.T) {
	source := &blockedPipeInput{reading: make(chan struct{})}
	body := newInvocationBody(t.Context(), cmdsdk.NewInput(source))
	done := make(chan error, 1)
	go func() { _, err := body.Read(make([]byte, 1)); done <- err }()
	<-source.reading
	// PipeClient.Put owns the body and closes it when a server answers early.
	if err := body.Close(); err != nil {
		t.Fatal(err)
	}
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatalf("pending input retained: %v", err)
	}
}
