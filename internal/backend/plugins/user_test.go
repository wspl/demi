package plugins_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// runCommand exercises a user toolset through its RPC boundary and captured IO.
func runCommand(ctx context.Context, set plugins.Toolset, path ...string) (string, uint8, error) {
	memory := hosttest.NewMemoryPort(nil)
	code, err := set.Commands.Dispatch(ctx, host.RPCInvocation{Path: path, Args: json.RawMessage(`{}`), Context: hosttest.CommandContext()}, memory.Port())
	return string(memory.Stdout()), code, err
}

func TestUserChoicesCommandsAndInstances(t *testing.T) {
	var created, closed atomic.Int32
	printer := func() plugin.Plugin {
		serial := created.Add(1)
		return &fakePlugin{close: func() { closed.Add(1) }, call: func(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
			call, ok := request.(*plugin.RequestCommand)
			if !ok {
				return (&fakePlugin{}).Call(ctx, request, port)
			}
			err := port.RPC().Stdout(ctx, []byte(fmt.Sprintf("%s:%d:%s", call.User, serial, strings.Join(call.Invocation.Path, " "))))
			return &plugin.ReplyExit{Code: 7}, err
		}}
	}
	a := manifest(t, "notes")
	a.Commands = []plugin.Commands{command("notes", plugin.PlacementDemi, nil), command("lint", plugin.PlacementRoot, nil)}
	a.Profiles = []core.Profile{{Name: "worker"}}
	b := manifest(t, "context-only")
	r := registry(t, &fakeFactory{manifest: a, make: printer}, &fakeFactory{manifest: b})
	u, shard := userFixture(t, r)
	registration := shard.sync.Register(shard.user, database.TokenHash{})
	defer registration.Release()
	product := host.Group("host", "Product.", host.Leaf(declare.Leaf[declare.NativeOperation]{Name: "list", Summary: "List.", Kind: &declare.RPC[declare.NativeOperation]{}}, host.RPCHandlerFunc(func(context.Context, host.RPCInvocation, host.RPCPort) (uint8, error) { return 9, nil })))
	set, err := u.Toolset(t.Context(), []host.Declared{product})
	if err != nil || set.Revision != "notes" || len(set.Profiles) != 1 {
		t.Fatalf("%+v %v", set, err)
	}
	for _, path := range [][]string{{"demi", "notes", "run"}, {"lint", "run"}} {
		out, code, err := runCommand(t.Context(), set, path...)
		wantPath := path
		if path[0] == "demi" {
			wantPath = path[1:]
		}
		if err != nil || code != 7 || out != string(shard.user)+":1:"+strings.Join(wantPath, " ") {
			t.Fatalf("%q %d %v", out, code, err)
		}
	}
	if _, code, err := runCommand(t.Context(), set, "demi", "host", "list"); err != nil || code != 9 {
		t.Fatalf("product: %d %v", code, err)
	}
	account, err := shard.control.Account(t.Context(), shard.user)
	if err != nil {
		t.Fatal(err)
	}
	secondAccount, err := shard.control.CreateUser(t.Context(), "second@example.test", account.PasswordHash, webapi.RoleUser)
	if err != nil {
		t.Fatal(err)
	}
	secondShard := &fakeShard{control: shard.control, user: secondAccount.ID}
	second := newUser(t, r, secondShard)
	secondSet, err := second.Toolset(t.Context(), nil)
	if err != nil {
		t.Fatal(err)
	}
	if out, _, err := runCommand(t.Context(), secondSet, "lint", "run"); err != nil || out != string(secondShard.user)+":2:lint run" {
		t.Fatalf("%s %v", out, err)
	}
	entries, err := u.Entries(t.Context())
	if err != nil || !entries[0].Enabled || entries[0].Name != a.Name || entries[0].Description != a.Description {
		t.Fatalf("%+v %v", entries, err)
	}
	if changed, err := u.Switch(t.Context(), "context-only", false); err != nil || !changed {
		t.Fatalf("%v %v", changed, err)
	}
	if revision, err := u.Revision(t.Context()); err != nil || revision != "notes" {
		t.Fatalf("%q %v", revision, err)
	}
	if changed, err := u.Switch(t.Context(), "notes", false); err != nil || !changed {
		t.Fatalf("%v %v", changed, err)
	}
	if closed.Load() != 1 {
		t.Fatal("disable did not close instance")
	}
	marks := registration.Take().Parts
	if !reflect.DeepEqual(marks, []pagesync.Part{{Kind: pagesync.Plugins}, {Kind: pagesync.Plugin, PluginID: "context-only"}, {Kind: pagesync.Plugin, PluginID: "notes"}}) {
		t.Fatal(marks)
	}
	if changed, err := u.Switch(t.Context(), "notes", false); err != nil || changed || len(registration.Take().Parts) != 0 {
		t.Fatalf("no-op: %v %v", changed, err)
	}
	fresh, err := u.Toolset(t.Context(), nil)
	if err != nil || fresh.Revision != "" || len(fresh.Profiles) != 0 || fresh.Commands.RenderHelp() != "" {
		t.Fatalf("%+v %v", fresh, err)
	}
	// Existing trees keep commands, but their calls cannot revive page access.
	if out, _, err := runCommand(t.Context(), set, "lint", "run"); err != nil || out != string(shard.user)+":3:lint run" {
		t.Fatalf("old tree: %s %v", out, err)
	}
	if state, err := u.PageState(t.Context(), "notes"); err != nil || state != nil {
		t.Fatalf("disabled page: %s %v", state, err)
	}
	if err := u.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	if closed.Load() != 2 {
		t.Fatal("close did not drop old-tree instance")
	}
	restored := newUser(t, r, shard)
	if revision, err := restored.Revision(t.Context()); err != nil || revision != "" {
		t.Fatalf("choice not persisted: %q %v", revision, err)
	}
	if changed, err := restored.Switch(t.Context(), "notes", true); err != nil || !changed {
		t.Fatalf("%v %v", changed, err)
	}
	if revision, err := restored.Revision(t.Context()); err != nil || revision != "notes" {
		t.Fatalf("%q %v", revision, err)
	}
	var switchError *plugins.SwitchError
	if _, err := restored.Switch(t.Context(), "missing", true); !errors.As(err, &switchError) || switchError.Err != nil {
		t.Fatalf("%v", err)
	}
}

func TestPageContextTopicsAndStreamLifecycle(t *testing.T) {
	m := manifest(t, "page")
	m.Streams = []plugin.Stream{{Name: "live", Operation: declare.NativeOperation{Package: "page", Operation: "live"}, Sends: m.Page.User.Schema, Receives: m.Page.User.Schema}}
	var requests []plugin.Request
	p := &fakePlugin{call: func(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
		requests = append(requests, request)
		if state, ok := request.(*plugin.RequestPageState); ok && state.Conversation != nil {
			if err := port.Changed(ctx, plugin.ScopeConversation); err != nil {
				return nil, err
			}
		}
		return (&fakePlugin{}).Call(ctx, request, port)
	}}
	u, shard := userFixture(t, registry(t, &fakeFactory{manifest: m, make: func() plugin.Plugin { return p }}))
	reg := shard.sync.Register(shard.user, database.TokenHash{})
	defer reg.Release()
	ctx := t.Context()
	conversation := webapi.ConversationID("conversation")
	ask := plugins.ContextAsk{Conversation: conversation, Node: "node", Cwd: "/work", Turn: "turn", Seen: []string{"oldest", "newest"}}
	if text, err := u.Context(ctx, "page", ask); err != nil || text == nil || *text != "news" {
		t.Fatalf("%v %v", text, err)
	}
	want := &plugin.RequestContext{User: shard.user, Conversation: conversation, Node: "node", CWD: "/work", Turn: "turn", Seen: ask.Seen}
	if !reflect.DeepEqual(requests[0], want) {
		t.Fatalf("%+v", requests[0])
	}
	if text, err := u.Context(ctx, "unknown", ask); err != nil || text != nil {
		t.Fatalf("%v %v", text, err)
	}
	if state, err := u.PageState(ctx, "unknown"); err != nil || state != nil {
		t.Fatalf("%v %v", state, err)
	}
	states, err := u.PageStates(ctx)
	if err != nil || string(states["page"]) != `{}` {
		t.Fatalf("%v %v", states, err)
	}
	u.Fire(plugin.TopicExposes, nil)
	u.Fire(plugin.TopicJobs, &conversation)
	other := webapi.ConversationID("other")
	if got := u.Revisions(other); len(got) != 1 || got[0].Revision != 0 {
		t.Fatal(got)
	}
	answer, err := u.ConversationState(ctx, "page", conversation)
	if err != nil || answer.Revision != 1 || string(answer.State) != `{}` || u.Revisions(conversation)[0].Revision != 2 {
		t.Fatalf("%+v %v", answer, err)
	}
	if got := reg.Take().Parts; !reflect.DeepEqual(got, []pagesync.Part{{Kind: pagesync.Conversation, ConversationID: conversation}, {Kind: pagesync.Plugin, PluginID: "page"}}) {
		t.Fatal(got)
	}
	raw := json.RawMessage(`{"text":"<&\u2028"}`)
	if result, err := u.PageCall(ctx, plugins.PageCall{Plugin: "page", Method: "conversation", Params: raw, Conversation: &conversation}); err != nil || string(result) != `{}` {
		t.Fatalf("%s %v", result, err)
	}
	last := requests[len(requests)-1].(*plugin.RequestPageCall)
	if string(last.Params) != string(raw) || last.User != shard.user || *last.Conversation != conversation {
		t.Fatalf("%+v", last)
	}
	cases := []struct {
		call plugins.PageCall
		kind plugins.PageCallErrorKind
	}{
		{plugins.PageCall{Plugin: "missing", Method: "user", Params: []byte(`{}`)}, plugins.UnknownPlugin},
		{plugins.PageCall{Plugin: "page", Method: "conversation", Params: []byte(`{}`)}, plugins.UnknownMethod},
		{plugins.PageCall{Plugin: "page", Method: "missing", Params: []byte(`{}`)}, plugins.UnknownMethod},
		{plugins.PageCall{Plugin: "page", Method: "user", Params: []byte(`{"text":1}`)}, plugins.InvalidParams},
		{plugins.PageCall{Plugin: "page", Method: "user", Params: []byte(`{"text":"x","text":"y"}`)}, plugins.InvalidParams},
		{plugins.PageCall{Plugin: "page", Method: "user", Params: []byte(`{broken`)}, plugins.InvalidParams},
	}
	before := len(requests)
	for _, tc := range cases {
		_, err := u.PageCall(ctx, tc.call)
		var refused *plugins.PageCallError
		if !errors.As(err, &refused) || refused.Kind != tc.kind {
			t.Fatalf("%v", err)
		}
	}
	if len(requests) != before {
		t.Fatal("invalid request reached plugin")
	}
	stream, err := u.StreamEnd(ctx, "live")
	if err != nil || stream == nil {
		t.Fatalf("%v %v", stream, err)
	}
	if missing, err := u.StreamEnd(ctx, "missing"); err != nil || missing != nil {
		t.Fatalf("%v %v", missing, err)
	}
	if _, err := u.Switch(ctx, "page", false); err != nil {
		t.Fatal(err)
	}
	select {
	case <-stream:
	default:
		t.Fatal("disable did not end stream")
	}
	if current, err := u.StreamEnd(ctx, "live"); err != nil || current != nil {
		t.Fatalf("%v %v", current, err)
	}
	if text, err := u.Context(ctx, "page", ask); err != nil || text != nil {
		t.Fatalf("%v %v", text, err)
	}
	if state, err := u.PageState(ctx, "page"); err != nil || state != nil {
		t.Fatalf("%s %v", state, err)
	}
	if states, err := u.PageStates(ctx); err != nil || len(states) != 0 {
		t.Fatalf("%v %v", states, err)
	}
	for _, read := range []func() error{
		func() error {
			_, err := u.PageCall(ctx, plugins.PageCall{Plugin: "page", Method: "user", Params: []byte(`{}`)})
			return err
		},
		func() error { _, err := u.ConversationState(ctx, "page", conversation); return err },
	} {
		var refused *plugins.PageCallError
		if err := read(); !errors.As(err, &refused) || refused.Kind != plugins.Disabled {
			t.Fatal(err)
		}
	}
	u.Fire(plugin.TopicJobs, &conversation)
	if got := u.Revisions(conversation); len(got) != 1 || got[0].Revision != 3 {
		t.Fatal(got)
	}
	if _, err := u.Switch(ctx, "page", true); err != nil {
		t.Fatal(err)
	}
	current, err := u.StreamEnd(ctx, "live")
	if err != nil || current == nil || current == stream {
		t.Fatalf("%v %v", current, err)
	}
	select {
	case <-current:
		t.Fatal("re-enabled stream is ended")
	default:
	}
	if err := u.Close(ctx); err != nil {
		t.Fatal(err)
	}
	select {
	case <-current:
	default:
		t.Fatal("shutdown did not end stream")
	}
}

func TestPluginFailuresStayObservable(t *testing.T) {
	for _, tc := range []struct {
		name    string
		reply   plugin.Reply
		failure error
	}{
		{name: "wrong reply", reply: &plugin.ReplyContext{}},
		{name: "plugin refusal", failure: &plugin.ErrorRefused{Reason: "busy", Message: "Busy"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			m := manifest(t, "fail")
			m.Commands = []plugin.Commands{command("fail", plugin.PlacementDemi, nil)}
			p := &fakePlugin{call: func(context.Context, plugin.Request, plugin.Port) (plugin.Reply, error) { return tc.reply, tc.failure }}
			u, _ := userFixture(t, registry(t, &fakeFactory{manifest: m, make: func() plugin.Plugin { return p }}))
			if _, err := u.PageState(t.Context(), "fail"); err == nil {
				t.Fatal("state failure lost")
			}
			_, err := u.PageCall(t.Context(), plugins.PageCall{Plugin: "fail", Method: "user", Params: []byte(`{}`)})
			var pageErr *plugins.PageCallError
			if !errors.As(err, &pageErr) || pageErr.Kind != plugins.PluginFailed {
				t.Fatal(err)
			}
			if tc.failure != nil && !errors.Is(err, tc.failure) {
				t.Fatal("plugin failure not wrapped")
			}
			set, err := u.Toolset(t.Context(), nil)
			if err != nil {
				t.Fatal(err)
			}
			if _, _, err := runCommand(t.Context(), set, "demi", "fail", "run"); err == nil {
				t.Fatal("command failure lost")
			}
		})
	}
}

func TestCommandErrorsPreserveClassificationAndCause(t *testing.T) {
	for _, failure := range []error{
		&plugin.ErrorUsage{Message: "bad arguments"},
		&host.PortError{Kind: host.PortEnded, Message: "caller left"},
		context.Canceled,
		errors.New("plugin broke"),
	} {
		t.Run(failure.Error(), func(t *testing.T) {
			m := manifest(t, "command")
			m.Commands = []plugin.Commands{command("command", plugin.PlacementDemi, nil)}
			p := &fakePlugin{call: func(context.Context, plugin.Request, plugin.Port) (plugin.Reply, error) { return nil, failure }}
			u, _ := userFixture(t, registry(t, &fakeFactory{manifest: m, make: func() plugin.Plugin { return p }}))
			set, err := u.Toolset(t.Context(), nil)
			if err != nil {
				t.Fatal(err)
			}
			_, _, err = runCommand(t.Context(), set, "demi", "command", "run")
			if !errors.Is(err, failure) {
				t.Fatalf("cause lost: %v", err)
			}
			switch failure.(type) {
			case *plugin.ErrorUsage:
				var rpc *host.RPCError
				if !errors.As(err, &rpc) || rpc.Kind != host.Usage {
					t.Fatal(err)
				}
			case *host.PortError:
				var port *host.PortError
				if !errors.As(err, &port) || port.Kind != host.PortEnded {
					t.Fatal(err)
				}
			}
		})
	}
}

func TestPageParametersMustBeAnObjectAndContextReplyMustMatch(t *testing.T) {
	m := manifest(t, "schema")
	schema, err := declare.NewSchema([]byte(`{}`))
	if err != nil {
		t.Fatal(err)
	}
	m.Page.Methods[0].Params = plugin.Schema{Schema: schema}
	p := &fakePlugin{call: func(context.Context, plugin.Request, plugin.Port) (plugin.Reply, error) {
		return &plugin.ReplyExit{}, nil
	}}
	u, _ := userFixture(t, registry(t, &fakeFactory{manifest: m, make: func() plugin.Plugin { return p }}))
	_, err = u.PageCall(t.Context(), plugins.PageCall{Plugin: "schema", Method: "user", Params: []byte(`[]`)})
	var invalid *plugins.PageCallError
	if !errors.As(err, &invalid) || invalid.Kind != plugins.InvalidParams || err.Error() != "the parameters are not an object" {
		t.Fatal(err)
	}
	_, err = u.Context(t.Context(), "schema", plugins.ContextAsk{Conversation: "conversation"})
	var failed *plugin.ErrorFailed
	if !errors.As(err, &failed) || !strings.Contains(err.Error(), "the plugin answered a context request with") {
		t.Fatal(err)
	}
	_, err = u.ConversationState(t.Context(), "absent", "conversation")
	if err == nil || err.Error() != `No plugin "absent"` {
		t.Fatal(err)
	}
	_, err = u.Switch(t.Context(), "absent", false)
	if err == nil || err.Error() != `No plugin "absent"` {
		t.Fatal(err)
	}
}
