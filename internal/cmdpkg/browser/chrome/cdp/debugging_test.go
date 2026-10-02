package cdp_test

import (
	"context"
	"encoding/json"
	"testing"
	"time"

	"github.com/chromedp/cdproto/target"
	"github.com/go-json-experiment/json/jsontext"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp/cdptest"
)

func TestDebuggingGenerationFiltersAndCallerCleanup(t *testing.T) {
	auto := target.SetAutoAttach(true, false).WithFlatten(true).WithFilter(target.Filter{{Type: "iframe"}, {Type: "worker"}, {Type: "shared_worker"}, {Type: "service_worker"}, {Exclude: true}})
	server := cdptest.NewServer(t,
		cdptest.Exchange{Method: "Target.attachToTarget", Params: target.AttachToTarget("tab").WithFlatten(true), Result: jsontext.Value(`{"sessionId":"one"}`)},
		cdptest.Exchange{Method: "Target.setAutoAttach", SessionID: "one", Params: auto, Result: struct{}{}},
		cdptest.Exchange{Method: "Runtime.getIsolateId", SessionID: "one", Params: struct{}{}, Result: jsontext.Value(`{"id":"barrier"}`)},
		cdptest.Exchange{Method: "Target.detachFromTarget", Params: target.DetachFromTarget().WithSessionID("one"), Result: struct{}{}},
		cdptest.Exchange{Method: "Target.attachToTarget", Params: target.AttachToTarget("tab").WithFlatten(true), Result: jsontext.Value(`{"sessionId":"two"}`)},
		cdptest.Exchange{Method: "Target.setAutoAttach", SessionID: "two", Params: auto, Result: struct{}{}},
	)
	debug := cdp.StartDebug(t.Context(), server.Address(), "tab")
	defer func() {
		if err := debug.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	handle, err := debug.Connect(t.Context(), 17)
	if err != nil {
		t.Fatal(err)
	}
	initial, err := debug.Events(t.Context(), cdp.EventQuery{Target: "main", Limit: 1})
	if err != nil {
		t.Fatal(err)
	}
	if len(initial.Result.Events) != 0 {
		t.Fatal(initial)
	}
	recorded := debug.Recorded()
	for range 2 {
		if err := server.Emit(t.Context(), cdp.Event{Method: "Runtime.consoleAPICalled", SessionID: "one", Params: json.RawMessage(`{"type":"log","args":[],"executionContextId":1,"timestamp":0}`)}); err != nil {
			t.Fatal(err)
		}
	}
	select {
	case <-recorded:
	case <-t.Context().Done():
		t.Fatal(t.Context().Err())
	}
	if _, err := handle.Send(t.Context(), "Runtime.getIsolateId", json.RawMessage(`{}`), "main"); err != nil {
		t.Fatal(err)
	}
	first, err := debug.Events(t.Context(), cdp.EventQuery{After: &initial.Result.Cursor, Target: "main", Methods: []string{"Runtime.consoleAPICalled"}, Limit: 1})
	if err != nil {
		t.Fatal(err)
	}
	if len(first.Result.Events) != 1 || !first.Result.HasMore {
		t.Fatal(first)
	}
	second, err := debug.Events(t.Context(), cdp.EventQuery{After: &first.Result.Cursor, Target: "main", Methods: []string{"Runtime.consoleAPICalled"}, Limit: 1})
	if err != nil {
		t.Fatal(err)
	}
	if len(second.Result.Events) != 1 || second.Result.HasMore || second.Result.Events[0].Sequence <= first.Result.Events[0].Sequence {
		t.Fatal(second)
	}
	if _, err := handle.Send(t.Context(), "Runtime.evaluate", json.RawMessage(`{"expression":"1"}`), "external"); cdp.ErrorCode(err) != "target_not_found" {
		t.Fatal(err)
	}
	if err := debug.Detach(t.Context(), 17); err != nil {
		t.Fatal(err)
	}
	if err := debug.Detach(t.Context(), 17); err != nil {
		t.Fatal(err)
	}
	if _, err := debug.Connect(t.Context(), 17); err != nil {
		t.Fatal(err)
	}
	_, err = debug.Events(t.Context(), cdp.EventQuery{After: &first.Result.Cursor, Target: "main", Limit: 1})
	requireCode(t, err, "stale_cursor")
}

func TestRawMethodAdmissionPrecedesConnection(t *testing.T) {
	debug := cdp.StartDebug(t.Context(), "ws://invalid.invalid", "tab")
	defer func() {
		if err := debug.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	for _, test := range []struct{ method, params, code string }{
		{"Browser.close", "{}", "cdp_method_denied"},
		{"Target.createTarget", "{}", "cdp_method_denied"},
		{"Page.close", "{}", "cdp_method_denied"},
		{"Runtime.missing", "{}", "invalid_input"},
		{"Runtime.evaluate", "{}", "invalid_input"},
		{"Runtime.evaluate", `{"expression":1}`, "invalid_input"},
		{"Runtime.evaluate", `{"expression":"1","sessionId":"external"}`, "invalid_input"},
	} {
		operation := cdp.NewOperation(t.Context(), t.Context(), time.Second)
		_, err := cdp.ExecuteCommand(t.Context(), operation, debug, 1, "t1", &browserop.CdpSendInput{Tab: "t1", Method: test.method, Params: test.params})
		operation.Close()
		requireCode(t, err, test.code)
	}
}
