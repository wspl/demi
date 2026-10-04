package claudecode

import (
	"context"
	"errors"
	"testing"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

type waitingInput struct{ started chan struct{} }

func (i waitingInput) Next(ctx context.Context) ([]byte, error) {
	close(i.started)
	<-ctx.Done()
	return nil, ctx.Err()
}

// Cancellation ends the invocation in each phase where it waits. Whether a
// document is written is not observable here: an Output whose context is
// cancelled refuses every write, so cmdsdk, not this package, guarantees that.
func TestCancellationEndsEveryPhase(t *testing.T) {
	for _, phase := range []string{"input", "install", "status"} {
		t.Run(phase, func(t *testing.T) {
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			artifacts, requests := cmdsdk.ArtifactsChannel()
			defer artifacts.Close()
			handler := &service{}
			handler.SetArtifacts(artifacts)
			output, _ := cmdsdk.OutputChannel(ctx)
			operation := "claude-code.ensure"
			started := make(chan struct{})
			var input cmdsdk.InputSource = &releaseInput{releaseRecord("2.1.278", currentPlatform())}
			if phase == "input" {
				input = waitingInput{started}
			}
			if phase == "status" {
				operation = "claude-code.status"
			}
			done := make(chan error, 1)
			go func() {
				_, err := handler.Invoke(
					ctx,
					cmdsdk.InvocationContext[commandwire.Invocation]{
						Request: commandwire.Invocation{Operation: operation, InvocationID: "invocation"},
						Input:   cmdsdk.NewInput(input),
						Output:  output,
					},
				)
				done <- err
			}()
			if phase == "input" {
				<-started
			} else {
				<-requests
			}
			cancel()
			if err := <-done; !errors.Is(err, context.Canceled) {
				t.Fatalf("cancellation: %v", err)
			}
		})
	}
}
