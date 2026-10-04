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
	code, err := set.Commands.Dispatch(
		ctx,
		host.RPCInvocation{
			Path:    path,
			Args:    json.RawMessage(`{}`),
			Context: hosttest.CommandContext(),
		},
		memory.Port(),
	)
	return string(memory.Stdout()), code, err
}

func TestUserChoicesCommandsAndInstances(t *testing.T) {
	var created, closed atomic.Int32
	printer := func() plugin.Plugin {
		serial := created.Add(1)
		return &fakePlugin{
			close: func() {
				closed.Add(1)
			},
			call: func(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
				call, ok := request.(*plugin.RequestCommand)
				if !ok {
					return (&fakePlugin{}).Call(ctx, request, port)
				}
				path := strings.Join(call.Invocation.Path, " ")
				text := fmt.Sprintf("%s:%d:%s", call.User, serial, path)
				err := port.RPC().Stdout(ctx, []byte(text))
				return &plugin.ReplyExit{Code: 7}, err
			},
		}
	}
	a := manifest(t, "notes")
	a.Commands = []plugin.Commands{
		command("notes", plugin.PlacementDemi, nil),
		command("lint", plugin.PlacementRoot, nil),
	}
	a.Profiles = []core.Profile{{Name: "worker"}}
	b := manifest(t, "context-only")
	r := registry(t, &fakeFactory{manifest: a, make: printer}, &fakeFactory{manifest: b})
	u, shard := userFixture(t, r)
	registration := shard.sync.Register(shard.user, database.TokenHash{})
	defer registration.Release()
	product := host.Group(
		"host",
		"Product.",
		host.Leaf(
			declare.Leaf[declare.NativeOperation]{
				Name:    "list",
				Summary: "List.",
				Kind:    &declare.RPC[declare.NativeOperation]{},
			},
			host.RPCHandlerFunc(
				func(context.Context, host.RPCInvocation, host.RPCPort) (uint8, error) { return 9, nil },
			),
		),
	)
	set, err := u.Toolset(t.Context(), []host.Declared{product})
	if err != nil || set.Revision != "notes" || len(set.Profiles) != 1 {
		t.Fatalf("%+v %v", set, err)
	}
	var roots []string
	for _, root := range set.Commands.Declarations() {
		roots = append(roots, declare.Name(root))
	}
	if !reflect.DeepEqual(roots, []string{"demi", "lint"}) {
		t.Fatal(roots)
	}
	for _, taught := range []string{"host: Product.", "notes: A group."} {
		if !strings.Contains(set.Commands.RenderHelp(), taught) {
			t.Fatal(set.Commands.RenderHelp())
		}
	}
	for _, path := range [][]string{{"demi", "notes", "run"}, {"lint", "run"}} {
		output, code, err := runCommand(t.Context(), set, path...)
		wantPath := path
		if path[0] == "demi" {
			wantPath = path[1:]
		}
		if err != nil || code != 7 || output != string(shard.user)+":1:"+strings.Join(wantPath, " ") {
			t.Fatalf("%q %d %v", output, code, err)
		}
	}
	if _, code, err := runCommand(t.Context(), set, "demi", "host", "list"); err != nil || code != 9 {
		t.Fatalf("product: %d %v", code, err)
	}
	account, _, err := shard.control.Account(t.Context(), shard.user)
	if err != nil {
		t.Fatal(err)
	}
	secondAccount, err := shard.control.CreateUser(
		t.Context(),
		"second@example.test",
		account.PasswordHash,
		webapi.RoleUser,
	)
	if err != nil {
		t.Fatal(err)
	}
	secondShard := &fakeShard{control: shard.control, user: secondAccount.ID}
	second := newUser(t, r, secondShard)
	secondSet, err := second.Toolset(t.Context(), nil)
	if err != nil {
		t.Fatal(err)
	}
	if output, _, err := runCommand(
		t.Context(),
		secondSet,
		"lint",
		"run",
	); err != nil ||
		output != string(secondShard.user)+":2:lint run" {
		t.Fatalf("%s %v", output, err)
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
	if !reflect.DeepEqual(
		marks,
		[]pagesync.Part{
			{Kind: pagesync.Plugins},
			{Kind: pagesync.Plugin, PluginID: "context-only"},
			{Kind: pagesync.Plugin, PluginID: "notes"},
		},
	) {
		t.Fatal(marks)
	}
	if changed, err := u.Switch(
		t.Context(),
		"notes",
		false,
	); err != nil || changed ||
		len(registration.Take().Parts) != 0 {
		t.Fatalf("no-op: %v %v", changed, err)
	}
	fresh, err := u.Toolset(t.Context(), nil)
	if err != nil || fresh.Revision != "" || len(fresh.Profiles) != 0 || fresh.Commands.RenderHelp() != "" {
		t.Fatalf("%+v %v", fresh, err)
	}
	// Existing trees keep commands, but their calls cannot revive page access.
	if output, _, err := runCommand(
		t.Context(),
		set,
		"lint",
		"run",
	); err != nil ||
		output != string(shard.user)+":3:lint run" {
		t.Fatalf("old tree: %s %v", output, err)
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
	if _, err := restored.Switch(t.Context(), "missing", true); !errors.Is(err, plugins.ErrUnknownPlugin) {
		t.Fatalf("%v", err)
	}
}

func TestPageContextTopicsAndStreamLifecycle(t *testing.T) {
	m := manifest(t, "page")
	m.Streams = []plugin.Stream{
		{
			Name:      "live",
			Operation: declare.NativeOperation{Package: "page", Operation: "live"},
			Sends:     m.Page.User.Schema,
			Receives:  m.Page.User.Schema,
		},
	}
	var requests []plugin.Request
	p := &fakePlugin{call: func(
		ctx context.Context,
		request plugin.Request,
		port plugin.Port,
	) (plugin.Reply, error) {
		requests = append(requests, request)
		if state, ok := request.(*plugin.RequestPageState); ok && state.Conversation != nil {
			if err := port.Changed(ctx, plugin.ScopeConversation); err != nil {
				return nil, err
			}
		}
		return (&fakePlugin{}).Call(ctx, request, port)
	}}
	u, shard := userFixture(t, registry(t, &fakeFactory{manifest: m, make: func() plugin.Plugin { return p }}))
	registration := shard.sync.Register(shard.user, database.TokenHash{})
	defer registration.Release()
	ctx := t.Context()
	conversation := webapi.ConversationID("conversation")
	ask := plugins.ContextAsk{
		Conversation: conversation,
		Node:         "node",
		Cwd:          "/work",
		Turn:         "turn",
		Seen:         []string{"oldest", "newest"},
	}
	if text, err := u.Context(ctx, "page", ask); err != nil || text == nil || *text != "news" {
		t.Fatalf("%v %v", text, err)
	}
	want := &plugin.RequestContext{
		User:         shard.user,
		Conversation: conversation,
		Node:         "node",
		CWD:          "/work",
		Turn:         "turn",
		Seen:         ask.Seen,
	}
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
	if err != nil || answer.Revision != 1 || string(answer.State) != `{}` ||
		u.Revisions(conversation)[0].Revision != 2 {
		t.Fatalf("%+v %v", answer, err)
	}
	if got := registration.Take().Parts; !reflect.DeepEqual(
		got,
		[]pagesync.Part{
			{Kind: pagesync.Conversation, ConversationID: conversation},
			{Kind: pagesync.Plugin, PluginID: "page"},
		},
	) {
		t.Fatal(got)
	}
	raw := json.RawMessage(`{"text":"<&\u2028"}`)
	if result, err := u.PageCall(
		ctx,
		plugins.PageCall{
			Plugin:       "page",
			Method:       "conversation",
			Params:       raw,
			Conversation: &conversation,
		},
	); err != nil ||
		string(result) != `{}` {
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
		{
			plugins.PageCall{
				Plugin: "page",
				Method: "conversation",
				Params: []byte(`{}`),
			},
			plugins.UnknownMethod,
		},
		{
			plugins.PageCall{Plugin: "page", Method: "missing", Params: []byte(`{}`)},
			plugins.UnknownMethod,
		},
		{
			plugins.PageCall{
				Plugin: "page",
				Method: "user",
				Params: []byte(`{"text":1}`),
			},
			plugins.InvalidParams,
		},
		{
			plugins.PageCall{
				Plugin: "page",
				Method: "user",
				Params: []byte(`{"text":"x","text":"y"}`),
			},
			plugins.InvalidParams,
		},
		{
			plugins.PageCall{Plugin: "page", Method: "user", Params: []byte(`{broken`)},
			plugins.InvalidParams,
		},
	}
	if _, err := u.PageCall(
		ctx,
		plugins.PageCall{Plugin: "missing", Method: "user", Params: []byte(`{}`)},
	); !errors.Is(
		err,
		plugins.ErrUnknownPlugin,
	) {
		t.Fatalf("%v", err)
	}
	before := len(requests)
	for _, scenario := range cases {
		_, err := u.PageCall(ctx, scenario.call)
		var refused *plugins.PageCallError
		if !errors.As(err, &refused) || refused.Kind != scenario.kind {
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
		func() error {
			_, err := u.ConversationState(ctx, "page", conversation)
			return err
		},
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
	for _, scenario := range []struct {
		name    string
		reply   plugin.Reply
		failure error
	}{
		{name: "wrong reply", reply: &plugin.ReplyContext{}},
		{
			name:    "plugin refusal",
			failure: &plugin.ErrorRefused{Reason: "busy", Message: "Busy"},
		},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			m := manifest(t, "fail")
			m.Commands = []plugin.Commands{command("fail", plugin.PlacementDemi, nil)}
			p := &fakePlugin{
				call: func(context.Context, plugin.Request, plugin.Port) (plugin.Reply, error) {
					return scenario.reply, scenario.failure
				},
			}
			u, _ := userFixture(t, registry(t, &fakeFactory{manifest: m, make: func() plugin.Plugin { return p }}))
			if _, err := u.PageState(t.Context(), "fail"); err == nil {
				t.Fatal("state failure lost")
			}
			_, err := u.PageCall(t.Context(), plugins.PageCall{Plugin: "fail", Method: "user", Params: []byte(`{}`)})
			var pageErr *plugins.PageCallError
			if !errors.As(err, &pageErr) || pageErr.Kind != plugins.PluginFailed {
				t.Fatal(err)
			}
			if scenario.failure != nil && !errors.Is(err, scenario.failure) {
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
			p := &fakePlugin{
				call: func(context.Context, plugin.Request, plugin.Port) (plugin.Reply, error) { return nil, failure },
			}
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
	if !errors.As(err, &invalid) || invalid.Kind != plugins.InvalidParams ||
		err.Error() != "the parameters are not an object" {
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

// A blocked plugin must not block panel answers; shutdown must cancel and join
// notifications. The plugin's port is restricted to its own kinds. No Host or
// model is used; the scenario's budget is one second.
func TestPanelNotificationsOwnWorkAndPortsOwnKinds(t *testing.T) {
	m := manifest(t, "panel")
	m.Page.PanelKinds = []string{"page"}
	m.Page.Told = []plugin.Topic{plugin.TopicJobs}
	other := manifest(t, "other")
	other.Page.PanelKinds = []string{"other"}
	admitted := make(chan plugin.Port, 1)
	ended := make(chan struct{})
	instance := &fakePlugin{
		call: func(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
			if _, ok := request.(*plugin.RequestPanelTab); ok {
				admitted <- port
				<-ctx.Done()
				close(ended)
				return nil, ctx.Err()
			}
			return &plugin.ReplyDone{}, nil
		},
	}
	u, shard := userFixture(
		t,
		registry(
			t,
			&fakeFactory{manifest: m, make: func() plugin.Plugin { return instance }},
			&fakeFactory{manifest: other},
		),
	)
	id := webapi.ConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01")
	if _, _, err := shard.control.CreateConversation(t.Context(), shard.user, id); err != nil {
		t.Fatal(err)
	}
	revision, err := u.ChangePanel(
		t.Context(),
		id,
		database.PanelCreate{Tab: webapi.CreatePanelTab{ID: "a", Kind: "page", Data: json.RawMessage(`{"z":1,"a":2}`)}},
	)
	if err != nil || revision != 1 {
		t.Fatalf("got %d %v; want revision 1", revision, err)
	}
	port := <-admitted
	// The original notification still waits; these operations must make progress.
	revision, err = port.UpdatePanelTab(t.Context(), "a", json.RawMessage(`{"b":3}`))
	if err != nil || revision != 2 {
		t.Fatalf("got %d %v; want revision 2", revision, err)
	}
	if _, _, _, err := shard.control.ChangePanel(
		t.Context(),
		id,
		database.PanelCreate{Tab: webapi.CreatePanelTab{ID: "b", Kind: "other", Data: json.RawMessage(`{}`)}},
	); err != nil {
		t.Fatal(err)
	}
	panel, err := port.PanelTabs(t.Context())
	if err != nil || len(panel.Tabs) != 1 || string(panel.Tabs[0].Data) != `{"z":1,"a":2,"b":3}` {
		t.Fatalf("got %+v %v; want only own tab preserving field order", panel, err)
	}
	if _, err := port.UpdatePanelTab(t.Context(), "a", json.RawMessage(`{"z":null}`)); err != nil {
		t.Fatal(err)
	}
	panel, err = port.PanelTabs(t.Context())
	if err != nil || string(panel.Tabs[0].Data) != `{"b":3,"a":2}` {
		t.Fatalf("got %+v %v; want removal to fill from the last field", panel, err)
	}
	_, err = port.RemovePanelTab(t.Context(), "b")
	var refusal *plugin.PortRefusalPanel
	if !errors.As(err, &refusal) || refusal.Code != webapi.ErrorCodeUnknownPanelKind {
		t.Fatalf("got %v; want unknown_panel_kind", err)
	}
	if err := u.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	<-ended
}
