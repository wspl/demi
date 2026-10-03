package cmdsdktest

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// ArtifactsFrom supplies artifact answers to local handlers with test-owned cleanup.
func ArtifactsFrom(
	t testing.TB,
	answer func(context.Context, commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error),
) *cmdsdk.Artifacts {
	t.Helper()
	a, requests := cmdsdk.ArtifactsChannel()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		defer close(done)
		for {
			select {
			case <-ctx.Done():
				return
			case p := <-requests:
				r, err := answer(ctx, p.Request)
				p.Reply(r, err)
			}
		}
	}()
	t.Cleanup(func() {
		cancel()
		a.Close()
		<-done
	})
	return a
}

// AnswerArtifacts opens and answers the service's artifact stream.
func AnswerArtifacts(
	ctx context.Context,
	client *cmdsdk.Client,
	answer func(context.Context, commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error),
) error {
	s, err := client.Artifacts(ctx)
	if err != nil {
		return err
	}
	return s.AnswerArtifacts(ctx, answer)
}
