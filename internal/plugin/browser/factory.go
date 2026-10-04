package browser

import (
	"bytes"
	"context"
	"encoding/json"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapiproto"
)

// Browser declares the browser commands, page and stream.
type Browser struct {
	commands *plugin.CommandPlugin
	page     plugin.Page
	stream   plugin.Stream
}

// New constructs the browser command and page declarations.
func New() (*Browser, error) {
	commands, err := commands()
	if err != nil {
		return nil, err
	}
	page, err := page()
	if err != nil {
		return nil, err
	}
	stream, err := LiveStream()
	if err != nil {
		return nil, err
	}
	return &Browser{commands: commands, page: page, stream: stream}, nil
}

// Manifest returns independently owned plugin declarations.
func (b *Browser) Manifest() plugin.Manifest {
	stream := b.stream
	stream.Constants = slices.Clone(stream.Constants)
	for i := range stream.Constants {
		stream.Constants[i].Value = bytes.Clone(stream.Constants[i].Value)
	}
	page := b.page
	page.PanelKinds = slices.Clone(page.PanelKinds)
	page.Told = slices.Clone(page.Told)
	state := *page.Conversation
	state.Topics = append([]plugin.Topic{}, state.Topics...)
	state.Operations = append([]commanddecl.NativeOperation{}, state.Operations...)
	page.Conversation = &state
	page.Methods = append([]plugin.Method{}, page.Methods...)
	for i := range page.Methods {
		page.Methods[i].Operations = append([]commanddecl.NativeOperation{}, page.Methods[i].Operations...)
	}
	return plugin.Manifest{
		ID:   "browser",
		Name: "Conversation browser",
		Description: "A browser on the conversation's Host that the agent drives with `demi browser` and " +
			"the user watches in the work panel.",
		Commands: b.commands.ManifestCommands(),
		Streams:  []plugin.Stream{stream},
		Page:     &page,
	}
}

// Instance creates one user's browser plugin.
func (b *Browser) Instance() plugin.Plugin {
	return &instance{commands: b.commands}
}

type instance struct {
	commands *plugin.CommandPlugin
	mu       sync.Mutex
	turns    map[webapiproto.ConversationID]chan struct{}
}

// Call dispatches browser commands and page requests.
func (i *instance) Call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	switch request := request.(type) {
	case *plugin.RequestCommand:
		return i.commands.Command(ctx, request.Invocation, port)
	case *plugin.RequestPageCall:
		if request.Conversation == nil {
			return nil, plugin.Undeclared("user method")
		}
		result, err := i.call(ctx, *request.Conversation, request.Method, request.Params, port)
		if err != nil {
			return nil, err
		}
		return &plugin.ReplyResult{Result: result}, nil
	case *plugin.RequestPageState:
		if request.Conversation == nil {
			return nil, plugin.Undeclared("user state")
		}
		state, err := tabs(ctx, port)
		if err != nil {
			return nil, err
		}
		return &plugin.ReplyState{State: state}, nil
	case *plugin.RequestPanelTab:
		var err error
		switch request.Change {
		case plugin.PanelTabCreated:
			err = i.bind(ctx, request.Conversation, port, request.Tab.ID)
		case plugin.PanelTabRemoved:
			err = i.removed(ctx, request.Conversation, port, request.Tab)
		}
		if err != nil {
			return nil, err
		}
		if err := port.Changed(ctx, plugin.ScopeConversation); err != nil {
			return nil, plugin.RequestError(err)
		}
		return &plugin.ReplyDone{}, nil
	case *plugin.RequestTopic:
		if request.Topic == plugin.TopicJobs && request.Conversation != nil {
			if err := i.syncTabs(ctx, *request.Conversation, port); err != nil {
				return nil, err
			}
			return &plugin.ReplyDone{}, nil
		}
		return nil, plugin.Undeclared("topic")
	case *plugin.RequestContext:
		return nil, plugin.Undeclared("context source")
	}
	return nil, plugin.Undeclared("request")
}

func page() (plugin.Page, error) {
	tabs, err := commanddecl.NewSchema(BrowserTabsPluginJSONSchema())
	if err != nil {
		return plugin.Page{}, err
	}
	page := plugin.Page{
		Package:    "@demicodes/plugin-browser",
		PanelKinds: []string{panelKind},
		Told:       []plugin.Topic{plugin.TopicJobs},
		Conversation: &plugin.State{
			Schema:     plugin.Schema{Schema: tabs},
			Topics:     []plugin.Topic{plugin.TopicJobs},
			Operations: []commanddecl.NativeOperation{operation("tabs")},
		},
	}
	nullResult := json.RawMessage(`{"title":"null","type":"null"}`)
	for _, spec := range []struct {
		name           string
		params, result json.RawMessage
		operations     []string
	}{
		{"bind", BindTabPluginJSONSchema(), nullResult, []string{"open", "goto", "close"}},
		{"sync", SyncTabsPluginJSONSchema(), nullResult, []string{"tabs"}},
		{"navigate", NavigateTabPluginJSONSchema(), nullResult, []string{"goto"}},
		{"history", TabHistoryPluginJSONSchema(), nullResult, []string{"back", "forward", "reload"}},
	} {
		params, err := commanddecl.NewSchema(spec.params)
		if err != nil {
			return plugin.Page{}, err
		}
		result, err := commanddecl.NewSchema(spec.result)
		if err != nil {
			return plugin.Page{}, err
		}
		method := plugin.Method{
			Name:   spec.name,
			Scope:  plugin.ScopeConversation,
			Params: plugin.Schema{Schema: params},
			Result: plugin.Schema{Schema: result},
		}
		for _, name := range spec.operations {
			method.Operations = append(method.Operations, operation(name))
		}
		page.Methods = append(page.Methods, method)
	}
	return page, nil
}
