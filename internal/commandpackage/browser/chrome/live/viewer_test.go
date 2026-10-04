package live

import (
	"context"
	"encoding/binary"
	"errors"
	"io"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
)

type viewerBrowser struct{ released, changed chan struct{} }

func (b *viewerBrowser) Running(ctx context.Context) (*tabs.Environment, *Hub, error) {
	return nil, nil, ctx.Err()
}

func (b *viewerBrowser) Changed() <-chan struct{} {
	return b.changed
}

func (b *viewerBrowser) Released() <-chan struct{} {
	return b.released
}

type viewerSource struct{ data chan []byte }

func (s *viewerSource) Next(ctx context.Context) ([]byte, error) {
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case data, ok := <-s.data:
		if !ok {
			return nil, io.EOF
		}
		return data, nil
	}
}

func moduleRecord(t *testing.T, record commandproto.Record) browserproto.LiveModuleMessage {
	t.Helper()
	data, ok := record.(commandproto.Stdout)
	if !ok {
		t.Fatalf("unexpected output %T", record)
	}
	if len(data) < 5 || int(binary.BigEndian.Uint32(data))+4 != len(data) {
		t.Fatalf("invalid framed output %x", data)
	}
	message, err := browserproto.DecodeLiveModuleMessage(data[5:])
	if err != nil {
		t.Fatal(err)
	}
	return message
}

func TestViewerRequiresHelloAndReportsProtocolRefusal(t *testing.T) {
	for _, data := range [][]byte{framed(browserproto.ControlFrame, []byte(`{"type":"release"}`)), {0, 0, 0, 0}} {
		ctx, cancel := context.WithCancel(t.Context())
		output, _ := commandsdk.OutputChannel(ctx)
		completion, err := Serve(
			ctx,
			&viewerBrowser{},
			commandsdk.InvocationContext[commandproto.Invocation]{
				Input:  commandsdk.NewInput(&chunks{[][]byte{data}}),
				Output: output,
			},
		)
		cancel()
		if err != nil || completion.ExitCode != 2 || completion.Error == nil ||
			completion.Error.Code != "invalid_input" {
			t.Fatalf("completion=%+v err=%v", completion, err)
		}
	}
}

func TestViewerWaitsWithoutBrowserAndEndsOnRelease(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		source := &viewerSource{data: make(chan []byte, 1)}
		source.data <- framed(browserproto.ControlFrame, []byte(`{"type":"hello","platform":"mac"}`))
		output, records := commandsdk.OutputChannel(ctx)
		browser := &viewerBrowser{released: make(chan struct{}), changed: make(chan struct{})}
		done := make(chan error, 1)
		go func() {
			completion, err := Serve(
				ctx,
				browser,
				commandsdk.InvocationContext[commandproto.Invocation]{
					Input:  commandsdk.NewInput(source),
					Output: output,
				},
			)
			if err == nil && completion.ExitCode != 0 {
				err = errors.New("nonzero completion")
			}
			done <- err
		}()
		message := moduleRecord(t, <-records)
		state, ok := message.(*browserproto.LiveModuleMessageState)
		if !ok || state.Running || len(state.Tabs) != 0 {
			t.Fatalf("initial state=%+v", message)
		}
		close(browser.released)
		message = moduleRecord(t, <-records)
		ended, ok := message.(*browserproto.LiveModuleMessageEnded)
		if !ok || ended.Reason != "released" {
			t.Fatalf("end=%+v", message)
		}
		if err := <-done; err != nil {
			t.Fatal(err)
		}
	})
}

func TestWriterHeartbeatAndCancellationWhileOutputBlocked(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		output, records := commandsdk.OutputChannel(ctx)
		w := startWriter(ctx, output)
		start := time.Now()
		message := moduleRecord(t, <-records)
		if _, ok := message.(*browserproto.LiveModuleMessageHeartbeat); !ok {
			t.Fatalf("heartbeat=%T", message)
		}
		if time.Since(start) != 250*time.Millisecond {
			t.Fatalf("heartbeat delay=%s", time.Since(start))
		}
		// The SDK's four records and writer's bounded controls fill while the page
		// does not read. Finish must cancel the blocked write and join it.
		for range 10 {
			w.control(ctx, &browserproto.LiveModuleMessageHeartbeat{})
		}
		synctest.Wait()
		start = time.Now()
		w.finish(ctx)
		if time.Since(start) != 5*time.Second {
			t.Fatalf("flush limit=%s", time.Since(start))
		}
	})
}
