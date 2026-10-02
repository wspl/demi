package cdp_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"testing"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/network"
	"github.com/chromedp/cdproto/page"
	"github.com/chromedp/cdproto/runtime"
	"github.com/chromedp/cdproto/target"
	"github.com/go-json-experiment/json/jsontext"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp/cdptest"
)

func ownedConnection(t *testing.T, address string) *cdp.Connection {
	t.Helper()
	connection, err := cdp.Dial(t.Context(), address)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := connection.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return connection
}

func TestTypedCDPAndLossDoNotBlockControlReplies(t *testing.T) {
	expression := "'<>&\u2028\u2029'"
	params := runtime.Evaluate(expression).WithContextID(42).WithReturnByValue(true)
	server := cdptest.NewServer(t,
		cdptest.Exchange{Method: "Runtime.evaluate", Params: params, Result: jsontext.Value(`{"result":{"type":"string","value":"<>&\u2028\u2029"}}`)},
		cdptest.Exchange{Method: "Runtime.getIsolateId", Params: struct{}{}, Result: jsontext.Value(`{"id":"barrier"}`)},
	)
	connection := ownedConnection(t, server.Address())
	subscription, err := connection.Subscribe("Runtime.consoleAPICalled")
	if err != nil {
		t.Fatal(err)
	}
	defer subscription.Close()
	result, exception, err := params.Do(protocol.WithExecutor(t.Context(), connection))
	if err != nil || exception != nil || result == nil {
		t.Fatalf("result=%v exception=%v err=%v", result, exception, err)
	}
	// Spell Unicode separators explicitly: line-based source edits must not change them.
	if want := "\"<>&\u2028\u2029\""; string(result.Value) != want {
		t.Fatalf("result value=%q, want %q", result.Value, want)
	}
	for range 19 {
		if err := server.Emit(t.Context(), cdp.Event{Method: "Runtime.consoleAPICalled", Params: json.RawMessage(`{"type":"log","args":[],"executionContextId":1,"timestamp":0}`)}); err != nil {
			t.Fatal(err)
		}
	}
	if err := connection.Execute(t.Context(), "Runtime.getIsolateId", struct{}{}, nil); err != nil {
		t.Fatal(err)
	}
	_, err = subscription.Next(t.Context())
	var loss *cdp.EventLoss
	if !errors.As(err, &loss) || loss.Count != 3 {
		t.Fatalf("loss=%v", err)
	}
	for range 16 {
		event, err := subscription.Next(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		if _, err := cdp.DecodeEvent(event); err != nil {
			t.Fatal(err)
		}
	}
}

func TestRendererChildrenRouteAndDetachBeforeTheirParent(t *testing.T) {
	auto := target.SetAutoAttach(true, false).WithFlatten(true).WithFilter(target.Filter{{Type: "iframe"}, {Type: "worker"}, {Type: "shared_worker"}, {Type: "service_worker"}, {Exclude: true}})
	server := cdptest.NewServer(t,
		cdptest.Exchange{Method: "Target.attachToTarget", Params: target.AttachToTarget("tab").WithFlatten(true), Result: jsontext.Value(`{"sessionId":"parent"}`)},
		cdptest.Exchange{Method: "Target.setAutoAttach", SessionID: "parent", Params: auto, Result: struct{}{}},
		cdptest.Exchange{Method: "Page.enable", SessionID: "child", Params: page.Enable(), Result: struct{}{}},
		cdptest.Exchange{Method: "Runtime.enable", SessionID: "child", Params: runtime.Enable(), Result: struct{}{}},
		cdptest.Exchange{Method: "Network.enable", SessionID: "child", Params: network.Enable(), Result: struct{}{}},
		cdptest.Exchange{Method: "Page.setLifecycleEventsEnabled", SessionID: "child", Params: page.SetLifecycleEventsEnabled(true), Result: struct{}{}},
		cdptest.Exchange{Method: "Target.setAutoAttach", SessionID: "child", Params: auto, Result: struct{}{}},
		cdptest.Exchange{Method: "Runtime.evaluate", SessionID: "child", Params: runtime.Evaluate("42").WithReturnByValue(true), Result: jsontext.Value(`{"result":{"type":"number","value":42}}`)},
		cdptest.Exchange{Method: "Page.getResourceContent", SessionID: "child", Params: page.GetResourceContent("frame", "https://example.test/image.svg"), Result: page.GetResourceContentReturns{Content: "<svg/>"}},
		cdptest.Exchange{Method: "Target.detachFromTarget", SessionID: "parent", Params: target.DetachFromTarget().WithSessionID("child"), Result: struct{}{}},
		cdptest.Exchange{Method: "Target.detachFromTarget", Params: target.DetachFromTarget().WithSessionID("parent"), Result: struct{}{}},
	)
	connection := ownedConnection(t, server.Address())
	session, err := connection.Attach(t.Context(), "tab")
	if err != nil {
		t.Fatal(err)
	}
	subscription, err := session.Subscribe("Target.attachedToTarget", "Runtime.consoleAPICalled")
	if err != nil {
		t.Fatal(err)
	}
	defer subscription.Close()
	event := cdp.Event{Method: "Target.attachedToTarget", SessionID: "parent", Params: json.RawMessage(`{"sessionId":"child","targetInfo":{"targetId":"frame","type":"iframe","title":"","url":"about:blank","attached":true,"canAccessOpener":false},"waitingForDebugger":false}`)}
	if err := server.Emit(t.Context(), event); err != nil {
		t.Fatal(err)
	}
	if _, err := subscription.Next(t.Context()); err != nil {
		t.Fatal(err)
	}
	child, err := session.Related(t.Context(), "frame")
	if err != nil || child == nil {
		t.Fatal(child, err)
	}
	result, _, err := runtime.Evaluate("42").WithReturnByValue(true).Do(protocol.WithExecutor(t.Context(), child))
	if err != nil || string(result.Value) != "42" {
		t.Fatal(result, err)
	}
	content, err := page.GetResourceContent("frame", "https://example.test/image.svg").Do(protocol.WithExecutor(t.Context(), child))
	if err != nil || string(content) != "<svg/>" {
		t.Fatalf("child resource=%q error=%v", content, err)
	}
	if err := server.Emit(t.Context(), cdp.Event{Method: "Runtime.consoleAPICalled", SessionID: "child", Params: json.RawMessage(`{"type":"log","args":[],"executionContextId":1,"timestamp":0}`)}); err != nil {
		t.Fatal(err)
	}
	received, err := subscription.Next(t.Context())
	if err != nil || received.SessionID != "child" {
		t.Fatal(received, err)
	}
	if err := session.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if err := child.Execute(t.Context(), "Runtime.getIsolateId", struct{}{}, nil); cdp.ErrorCode(err) != "tab_not_found" {
		t.Fatal(err)
	}
}

func TestOversizedChromeMessageLosesConnectionAndCleanupJoins(t *testing.T) {
	server := cdptest.NewServer(t, cdptest.Exchange{Method: "Runtime.evaluate", Params: struct{}{}, Result: map[string]any{"payload": strings.Repeat("x", int(cdp.MessageLimit))}})
	connection := ownedConnection(t, server.Address())
	err := connection.Execute(t.Context(), "Runtime.evaluate", struct{}{}, nil)
	requireCode(t, err, "browser_lost")
	if err := connection.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if connection.Err() == nil {
		t.Fatal("lost cause discarded")
	}
}

func TestTypedReplyValidationRequiresFieldsButToleratesChromeAdditions(t *testing.T) {
	params := runtime.Evaluate("1")
	server := cdptest.NewServer(t,
		cdptest.Exchange{Method: "Runtime.evaluate", Params: params, Result: jsontext.Value(`{"result":{"type":"number","value":1,"futureField":true},"futureEnvelopeField":true}`)},
		cdptest.Exchange{Method: "Runtime.evaluate", Params: params, Result: struct{}{}},
	)
	connection := ownedConnection(t, server.Address())
	result, _, err := params.Do(protocol.WithExecutor(t.Context(), connection))
	if err != nil || result == nil || string(result.Value) != "1" {
		t.Fatal(result, err)
	}
	_, _, err = params.Do(protocol.WithExecutor(t.Context(), connection))
	if err == nil {
		t.Fatal("absent required result accepted")
	}
	requireCode(t, err, "driver_error")
}

// A command reply is a wire barrier: all preceding events have reached the
// unread subscription. Local WebSocket only; budget below one second.
func TestSubscriptionCapacityPerEventType(t *testing.T) {
	for _, capacity := range []int{16, 256} {
		for _, overflow := range []bool{false, true} {
			t.Run(fmt.Sprintf("%d/overflow=%v", capacity, overflow), func(t *testing.T) {
				server := cdptest.NewServer(t, cdptest.Exchange{Method: "Runtime.getIsolateId", Params: struct{}{}, Result: jsontext.Value(`{"id":"barrier"}`)})
				connection := ownedConnection(t, server.Address())
				methods := []string{"Page.frameStartedLoading", "Page.frameStoppedLoading"}
				var sub *cdp.Subscription
				var err error
				if capacity == 16 {
					sub, err = connection.Subscribe(methods...)
				} else {
					sub, err = connection.SubscribeWithCapacity(capacity, methods...)
				}
				if err != nil {
					t.Fatal(err)
				}
				defer sub.Close()
				count := capacity
				if overflow {
					count++
				}
				for i := range count {
					for _, method := range methods {
						if err := server.Emit(t.Context(), cdp.Event{Method: method, Params: json.RawMessage(fmt.Sprintf(`{"frameId":"%d"}`, i))}); err != nil {
							t.Fatal(err)
						}
					}
				}
				if err := connection.Execute(t.Context(), "Runtime.getIsolateId", struct{}{}, nil); err != nil {
					t.Fatal(err)
				}
				start := 0
				if overflow {
					_, err := sub.Next(t.Context())
					var loss *cdp.EventLoss
					if !errors.As(err, &loss) || loss.Count != 2 {
						t.Fatalf("loss = %v; want two overwritten events", err)
					}
					start = 1
				}
				for i := start; i < count; i++ {
					for _, method := range methods {
						event, err := sub.Next(t.Context())
						if err != nil || event.Method != method || string(event.Params) != fmt.Sprintf(`{"frameId":"%d"}`, i) {
							t.Fatalf("event = %+v, err = %v; want %s frame %d", event, err, method, i)
						}
					}
				}
			})
		}
	}
}

// Ports chromiumoxide's capacity_is_bounded; no transport or waiting.
func TestSubscriptionCapacityBounds(t *testing.T) {
	var connection cdp.Connection
	for _, capacity := range []int{0, 257} {
		if _, err := connection.SubscribeWithCapacity(capacity, "Page.frameStartedLoading"); err == nil {
			t.Fatalf("accepted capacity %d", capacity)
		}
	}
}
