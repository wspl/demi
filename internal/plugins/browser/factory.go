package browser

import (
	"bytes"
	"context"
	"encoding/json"
	"slices"

	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
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
	state := *page.Conversation
	state.Topics = append([]plugin.Topic{}, state.Topics...)
	state.Operations = append([]declare.NativeOperation{}, state.Operations...)
	page.Conversation = &state
	page.Methods = append([]plugin.Method{}, page.Methods...)
	for i := range page.Methods {
		page.Methods[i].Operations = append([]declare.NativeOperation{}, page.Methods[i].Operations...)
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
func (b *Browser) Instance() plugin.Plugin { return &instance{commands: b.commands} }

type instance struct{ commands *plugin.CommandPlugin }

// Call dispatches browser commands and page requests.
func (i *instance) Call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	switch request := request.(type) {
	case *plugin.RequestCommand:
		return i.commands.Command(ctx, request.Invocation, port)
	case *plugin.RequestPageCall:
		result, err := call(ctx, request.Method, request.Params, port)
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
	case *plugin.RequestContext:
		return nil, plugin.Undeclared("context source")
	}
	return nil, plugin.Undeclared("request")
}

func page() (plugin.Page, error) {
	tabs, err := declare.NewSchema(BrowserTabsPluginJSONSchema())
	if err != nil {
		return plugin.Page{}, err
	}
	page := plugin.Page{
		Package: "@demicodes/plugin-browser",
		Conversation: &plugin.State{
			Schema:     plugin.Schema{Schema: tabs},
			Topics:     []plugin.Topic{plugin.TopicJobs},
			Operations: []declare.NativeOperation{operation("tabs")},
		},
	}
	nullResult := json.RawMessage(`{"title":"null","type":"null"}`)
	for _, spec := range []struct {
		name           string
		params, result json.RawMessage
		operations     []string
	}{
		{"open", OpenTabPluginJSONSchema(), OpenedTabPluginJSONSchema(), []string{"open"}},
		{"close", CloseTabPluginJSONSchema(), nullResult, []string{"close"}},
		{"navigate", NavigateTabPluginJSONSchema(), nullResult, []string{"goto"}},
		{"history", TabHistoryPluginJSONSchema(), nullResult, []string{"back", "forward", "reload"}},
	} {
		params, err := declare.NewSchema(spec.params)
		if err != nil {
			return plugin.Page{}, err
		}
		result, err := declare.NewSchema(spec.result)
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
