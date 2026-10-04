package browser_test

import (
	"context"
	"errors"
	"net"
	"strings"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
)

type sweepShutdownHandler struct {
	artifacts *commandsdk.Artifacts
	installed []commandproto.InstalledArtifact
}

// Operations returns the fixture operation name.
func (h *sweepShutdownHandler) Operations() []string {
	return []string{"test"}
}

// SetArtifacts attaches the fixture artifact stream.
func (h *sweepShutdownHandler) SetArtifacts(a *commandsdk.Artifacts) {
	h.artifacts = a
}

// Invoke completes the fixture invocation.
func (h *sweepShutdownHandler) Invoke(
	context.Context,
	commandsdk.InvocationContext[commandproto.Invocation],
) (commandproto.Completion, error) {
	return commandproto.Completion{}, nil
}

// Close requests installed artifacts before completing shutdown.
func (h *sweepShutdownHandler) Close(ctx context.Context) error {
	var err error
	h.installed, err = h.artifacts.Installed(ctx, "chrome")
	return err
}

// The browser's startup sweep needs artifact answers while its close hook joins it.
// This wire regression costs no processes or wall-clock waits.
func TestShutdownKeepsHandlerArtifactStreamUntilCloseEnds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		left, right := net.Pipe()
		h := &sweepShutdownHandler{}
		served := make(chan error, 1)
		go func() {
			served <- commandsdk.Serve(ctx, right, h)
			close(served)
		}()
		t.Cleanup(func() {
			cancel()
			if err := left.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
				t.Error(err)
			}
			<-served
		})
		client, err := commandsdk.Connect(ctx, left)
		if err != nil {
			t.Fatal(err)
		}
		stream, err := client.Artifacts(ctx)
		if err != nil {
			t.Fatal(err)
		}
		requested := make(chan struct{})
		release := make(chan struct{})
		answered := make(chan error, 1)
		go func() {
			answered <- stream.AnswerArtifacts(ctx, func(
				ctx context.Context,
				q commandproto.ArtifactRequest,
			) (commandproto.ArtifactAnswer, error) {
				close(requested)
				select {
				case <-release:
				case <-ctx.Done():
					return commandproto.ArtifactAnswer{}, ctx.Err()
				}
				installed := []commandproto.InstalledArtifact{
					{Version: "1", SHA256: strings.Repeat("a", 64), Path: "/runner/chrome"},
				}
				return commandproto.ArtifactAnswer{ID: q.ID, Installed: &installed}, nil
			})
		}()
		t.Cleanup(func() {
			cancel()
			<-answered
		})
		if err := client.Shutdown(ctx); err != nil {
			t.Fatal(err)
		}
		synctest.Wait()
		select {
		case <-requested:
		default:
			t.Fatal("handler could not request artifacts during close")
		}
		select {
		case err := <-served:
			t.Fatalf("service ended before its handler: %v", err)
		default:
		}
		close(release)
		if err := <-served; err != nil {
			t.Fatal(err)
		}
		err = <-answered
		answered <- err
		if err != nil {
			t.Fatal(err)
		}
		if len(h.installed) != 1 || h.installed[0].Path != "/runner/chrome" {
			t.Fatalf("close lost its artifact answer: %v", h.installed)
		}
	})
}
