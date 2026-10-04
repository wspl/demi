package tabs

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/contract"
)

// A replacement extension must end old captures, and closing an old consumer
// must not stop a capture on the replacement connection. All waits are protocol
// events; the environment cleanup joins the listener and both socket directions.
func TestCaptureReplacementAndCloseOwnership(t *testing.T) {
	ctx := t.Context()
	channel, address, closeChannel, err := bindCapture(ctx)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := closeChannel(); err != nil {
			t.Error(err)
		}
	})
	badAddress := strings.Split(address, "?")[0] + "?token=wrong"
	bad, response, err := websocket.Dial(ctx, badAddress, nil)
	if bad != nil {
		_ = bad.CloseNow()
	}
	if err == nil || response == nil || response.StatusCode != http.StatusForbidden {
		t.Fatalf("authentication response %v: %v", response, err)
	}
	dial := func() *websocket.Conn {
		socket, _, err := websocket.Dial(ctx, address, nil)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() {
			_ = socket.CloseNow()
		})
		return socket
	}
	read := func(socket *websocket.Conn) browserproto.CaptureCommand {
		kind, data, err := socket.Read(ctx)
		if err != nil {
			t.Fatal(err)
		}
		if kind != websocket.MessageText {
			t.Fatal(kind)
		}
		command, err := browserproto.DecodeCaptureCommand(data)
		if err != nil {
			t.Fatal(err)
		}
		return command
	}
	first := dial()
	old, err := channel.Start(ctx, "old", 700, 500, 30, 2_000_000)
	var capability *cdp.BrowserError
	if reason := captureUnavailable(); reason != "" && errors.As(err, &capability) &&
		capability.Kind == cdp.KindUnsupportedCapability &&
		capability.Message == reason {
		t.Skipf("Chrome capture unavailable: %s", reason)
	}
	if err != nil {
		t.Fatal(err)
	}
	command, ok := read(first).(*browserproto.CaptureCommandStart)
	if !ok || command.Target != "old" {
		t.Fatalf("start: %#v", command)
	}
	data, err := contract.EncodeJSON(&browserproto.CaptureEventStarted{Capture: command.Capture})
	if err != nil {
		t.Fatal(err)
	}
	if err := first.Write(ctx, websocket.MessageText, data); err != nil {
		t.Fatal(err)
	}
	event, ok := <-old.Events()
	if _, started := event.(*CaptureStarted); !started || !ok {
		t.Fatalf("started: %#v %v", event, ok)
	}
	second := dial()
	event, ok = <-old.Events()
	if _, failed := event.(*CaptureFailed); !failed || !ok {
		t.Fatalf("replacement: %#v %v", event, ok)
	}
	current, err := channel.Start(ctx, "new", 700, 500, 30, 2_000_000)
	if err != nil {
		t.Fatal(err)
	}
	replacement, ok := read(second).(*browserproto.CaptureCommandStart)
	if !ok || replacement.Target != "new" || replacement.Capture == command.Capture {
		t.Fatalf("replacement start: %#v", replacement)
	}
	if err := old.Close(ctx); err != nil {
		t.Fatal(err)
	}
	current.Ack(7, 2)
	ack, ok := read(second).(*browserproto.CaptureCommandAck)
	if !ok || ack.Capture != replacement.Capture || ack.Sequence != 7 {
		t.Fatalf("ack: %#v", ack)
	}
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	if err := current.Close(cancelled); err != context.Canceled {
		t.Fatal(err)
	}
	stop, ok := read(second).(*browserproto.CaptureCommandStop)
	if !ok || stop.Capture != replacement.Capture {
		t.Fatalf("stop: %#v", stop)
	}
	if _, ok := <-current.Events(); ok {
		t.Fatal("Close returned without releasing its event queue")
	}
	if err := current.Close(ctx); err != nil {
		t.Fatal(err)
	}
}
