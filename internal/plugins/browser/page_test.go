package browser_test

import (
	"context"
	"encoding/json"
	"errors"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/plugintest"
	"github.com/wspl/demi/internal/plugins/browser"
	"github.com/wspl/demi/internal/webapi"
)

func world(t *testing.T, answer plugintest.PackageCalls) (plugin.Plugin, *plugintest.TestDemi) {
	t.Helper()
	factory, err := browser.New()
	if err != nil {
		t.Fatal(err)
	}
	demi := plugintest.New()
	demi.PackageCalls = answer
	demi.Panel = panelTransport(func(context.Context, plugin.PortMessage) (plugin.PortAnswer, error) {
		return &plugin.PortAnswerPanel{Panel: webapi.EmptyWorkPanel()}, nil
	})
	return plugintest.Loopback(factory.Instance()), demi
}

func pageCall(
	t *testing.T,
	p plugin.Plugin,
	demi *plugintest.TestDemi,
	method, params string,
) (json.RawMessage, error) {
	t.Helper()
	reply, err := p.Call(
		t.Context(),
		&plugin.RequestPageCall{
			User:         "u1",
			Conversation: new(webapi.ConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b")),
			Method:       method,
			Params:       json.RawMessage(params),
		},
		demi.Port(),
	)
	if err != nil {
		return nil, err
	}
	result, ok := reply.(*plugin.ReplyResult)
	if !ok {
		t.Fatalf("reply: %T", reply)
	}
	return result.Result, nil
}

func pageState(t *testing.T, p plugin.Plugin, demi *plugintest.TestDemi) json.RawMessage {
	t.Helper()
	reply, err := p.Call(
		t.Context(),
		&plugin.RequestPageState{
			User:         "u1",
			Conversation: new(webapi.ConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b")),
		},
		demi.Port(),
	)
	if err != nil {
		t.Fatal(err)
	}
	state, ok := reply.(*plugin.ReplyState)
	if !ok {
		t.Fatalf("reply: %T", reply)
	}
	return state.State
}

func TestEachMethodCallsItsOperationWithoutWakingHost(t *testing.T) {
	p, demi := world(
		t,
		func(
			_ context.Context,
			operation declare.NativeOperation,
			args json.RawMessage,
			_ plugin.CallKind,
		) (json.RawMessage, error) {
			if operation.Package != "demi.browser" {
				t.Fatalf("package: %s", operation.Package)
			}
			switch operation.Operation {
			case "browser.open":
				if string(args) != `{"url":"about:blank"}` {
					t.Fatalf("open args: %s", args)
				}
				return json.RawMessage(`{"tab":"t1","url":"about:blank"}`), nil
			case "browser.tabs":
				return json.RawMessage(
					`{"tabs":[{"id":"t1","title":"","url":"about:blank","createdBy":{"kind":"user"}}],"truncated":false}`,
				), nil
			default:
				return json.RawMessage(`{}`), nil
			}
		},
	)
	if listed := pageState(
		t,
		p,
		demi,
	); string(
		listed,
	) != `{"tabs":[{"id":"t1","title":"","url":"about:blank","createdBy":{"kind":"user"}}]}` {
		t.Fatalf("tabs: %s", listed)
	}

	for _, test := range []struct{ method, params string }{
		{"navigate", `{"tab":"t1","url":"https://example.test/"}`},
		{"history", `{"tab":"t1","action":"back"}`},
		{"history", `{"tab":"t1","action":"reload"}`},
		{"sync", `{}`},
	} {
		result, err := pageCall(t, p, demi, test.method, test.params)
		if err != nil || string(result) != "null" {
			t.Fatalf("%s: %s, %v", test.method, result, err)
		}
	}
	if demi.Changes() != 4 {
		t.Fatalf("changes: %d", demi.Changes())
	}
	var got []string
	for _, call := range demi.Called() {
		got = append(got, call.Operation.Operation+":"+string(call.Kind))
	}
	wantCalls := []string{
		"browser.tabs:looks",
		"browser.goto:operates",
		"browser.back:operates",
		"browser.reload:operates",
		"browser.tabs:looks",
	}
	if diff := cmp.Diff(wantCalls, got); diff != "" {
		t.Fatal(diff)
	}
}

func TestStoppedHostAndMissingTabs(t *testing.T) {
	stopped := &plugin.PortRefusalHost{Code: webapi.ErrorCodeHostStopped, Status: 409, Message: "The Cloud is stopped"}
	p, demi := world(
		t,
		func(
			_ context.Context,
			operation declare.NativeOperation,
			_ json.RawMessage,
			_ plugin.CallKind,
		) (json.RawMessage, error) {
			switch operation.Operation {
			case "browser.tabs", "browser.goto":
				return nil, stopped
			default:
				return nil, &plugin.PortRefusalOperation{
					Stderr: `{"error":{"code":"tab_not_found","message":"No tab t9"}}`,
				}
			}
		},
	)
	demi.Panel = panelTransport(func(context.Context, plugin.PortMessage) (plugin.PortAnswer, error) {
		t.Error("stopped Host sync reached the panel; want saved tabs unchanged")
		return &plugin.PortAnswerPanel{Panel: webapi.EmptyWorkPanel()}, nil
	})
	if got := pageState(t, p, demi); string(got) != `{"tabs":[]}` {
		t.Fatalf("tabs: %s", got)
	}
	if _, err := pageCall(t, p, demi, "sync", `{}`); err != nil {
		t.Fatal(err)
	}
	for _, params := range []string{`{"tab":"t9","action":"forward"}`, `{"tab":"not-a-tab","action":"back"}`} {
		_, err := pageCall(t, p, demi, "history", params)
		var refused *plugin.ErrorRefused
		if !errors.As(err, &refused) || refused.Reason != "tab_not_found" {
			t.Fatalf("history: %v", err)
		}
	}
	_, err := pageCall(t, p, demi, "navigate", `{"tab":"t9","url":"https://example.test/"}`)
	var refusal *plugin.PortRefusalHost
	if !errors.As(err, &refusal) || cmp.Diff(stopped, refusal) != "" {
		t.Fatalf("navigate: %v", err)
	}
	if demi.Changes() != 1 {
		t.Fatalf("changes: %d", demi.Changes())
	}
}

func TestPageValidationAndRefusalsDoNotMarkChanges(t *testing.T) {
	for _, test := range []struct {
		name, method, params string
		answer               json.RawMessage
		failure              error
		reason               string
		usage                bool
	}{
		{name: "unknown argument", method: "bind", params: `{"extra":true}`, usage: true},
		{name: "empty URL", method: "navigate", params: `{"tab":"t1","url":""}`, usage: true},
		{name: "unknown history action", method: "history", params: `{"tab":"t1","action":"home"}`, usage: true},
		{
			name:   "invalid navigation tab",
			method: "navigate",
			params: `{"tab":"bad","url":"https://example.test"}`,
			reason: "tab_not_found",
		},
		{
			name:   "browser refusal",
			method: "navigate",
			params: `{"tab":"t1","url":"https://example.test"}`,
			failure: &plugin.PortRefusalOperation{
				Stderr: ` {"error":{"code":"tab_busy","message":"busy"}} `,
			},
			reason: "tab_busy",
		},

		{
			name:   "unstructured failure",
			method: "history",
			params: `{"tab":"t1","action":"back"}`,
			failure: &plugin.PortRefusalOperation{
				Stderr: "browser stopped unexpectedly",
			},
		},
	} {
		t.Run(test.name, func(t *testing.T) {
			p, demi := world(
				t,
				func(context.Context, declare.NativeOperation, json.RawMessage, plugin.CallKind) (json.RawMessage, error) {
					return test.answer, test.failure
				},
			)
			_, err := pageCall(t, p, demi, test.method, test.params)
			if err == nil {
				t.Fatal("call succeeded")
			}
			if test.usage {
				var usage *plugin.ErrorUsage
				if !errors.As(err, &usage) {
					t.Fatalf("usage: %v", err)
				}
			} else if test.reason != "" {
				var refusal *plugin.ErrorRefused
				if !errors.As(err, &refusal) || refusal.Reason != test.reason {
					t.Fatalf("refusal: %v", err)
				}
			} else {
				var refusal *plugin.PortRefusalOperation
				if !errors.As(err, &refusal) {
					t.Fatalf("operation: %v", err)
				}
			}
			if demi.Changes() != 0 {
				t.Fatal("failed method marked state changed")
			}
		})
	}
}
