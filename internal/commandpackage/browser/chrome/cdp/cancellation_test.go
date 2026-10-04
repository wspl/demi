package cdp_test

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/chromedp/cdproto"
	"github.com/coder/websocket"
	jsonv2 "github.com/go-json-experiment/json"
	"github.com/go-json-experiment/json/jsontext"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
)

func TestCancelledRawSendDetachesItsSessionAndJoinsSocket(t *testing.T) {
	started := make(chan struct{})
	detached := make(chan struct{})
	ended := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer close(ended)
		socket, err := websocket.Accept(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		defer func() { _ = socket.CloseNow() }()
		socket.SetReadLimit(cdp.MessageLimit)
		for {
			_, data, err := socket.Read(t.Context())
			if err != nil {
				return
			}
			var command cdproto.Message
			if err := jsonv2.Unmarshal(data, &command); err != nil {
				t.Error(err)
				return
			}
			result := jsontext.Value(`{}`)
			switch string(command.Method) {
			case "Target.attachToTarget":
				result = jsontext.Value(`{"sessionId":"owned"}`)
			case "Target.setAutoAttach":
			case "Runtime.evaluate":
				close(started)
				continue // Chrome is paused; cancellation must still detach.
			case "Target.detachFromTarget":
				close(detached)
			default:
				t.Errorf("unexpected CDP method %s", command.Method)
				return
			}
			reply := map[string]any{"id": command.ID, "result": result}
			if command.SessionID != "" {
				reply["sessionId"] = command.SessionID
			}
			data, err = jsonv2.Marshal(reply)
			if err != nil {
				t.Error(err)
				return
			}
			if err := socket.Write(t.Context(), websocket.MessageText, data); err != nil {
				return
			}
		}
	}))
	defer server.Close()
	debug := cdp.StartDebug(t.Context(), "ws"+strings.TrimPrefix(server.URL, "http"), "tab")
	defer func() {
		if err := debug.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	operation := cdp.NewOperation(ctx, t.Context(), time.Minute)
	defer operation.Close()
	result := make(chan error, 1)
	go func() {
		_, err := cdp.ExecuteCommand(
			ctx,
			operation,
			debug,
			1,
			"t1",
			&browserproto.CDPSendInput{
				Tab:    "t1",
				Method: "Runtime.evaluate",
				Params: `{"expression":"debugger; 42","returnByValue":true}`,
			},
		)
		result <- err
	}()
	<-started
	cancel()
	requireCode(t, <-result, "cancelled")
	if callers := debug.OtherCallers(nil); len(callers) != 0 {
		t.Fatal(callers)
	}
	<-detached
	<-ended
}

func TestUnavailableWebMCPDoesNotInstallPageHooks(t *testing.T) {
	executor := &capabilityExecutor{}
	operation := cdp.NewOperation(t.Context(), t.Context(), time.Second)
	defer operation.Close()
	_, err := cdp.ExecuteWebMCP(
		t.Context(),
		operation,
		executor,
		&cdp.WebMCPState{},
		"t1",
		&browserproto.WebMCPListInput{Tab: "t1"},
	)
	requireCode(t, err, "unsupported_capability")
	if executor.calls != 1 {
		t.Fatal("unavailable WebMCP performed page work", executor.calls)
	}
}

type capabilityExecutor struct{ calls int }

func (e *capabilityExecutor) Execute(_ context.Context, method string, _ any, result any) error {
	e.calls++
	if method != "Runtime.evaluate" {
		return &cdp.BrowserError{Kind: cdp.KindCDP, Message: "unexpected capability command"}
	}
	return jsonv2.Unmarshal(json.RawMessage(`{"result":{"type":"boolean","value":false}}`), result)
}
