package browser

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapiproto"
)

func listTabs(ctx context.Context, port plugin.Port) ([]browserproto.BrowserTab, error) {
	result, err := run(ctx, port, &browserproto.TabsInput{}, plugin.CallKindLooks)
	if err != nil {
		return nil, refused(err)
	}
	listed, err := browserproto.DecodeTabsResult(result)
	if err != nil {
		return nil, unreadable(err)
	}
	return listed.Tabs, nil
}

func tabs(ctx context.Context, port plugin.Port) (json.RawMessage, error) {
	listed, err := listTabs(ctx, port)
	if stopped(err) {
		listed = []browserproto.BrowserTab{}
	} else if err != nil {
		return nil, err
	}
	return contract.EncodeJSON(BrowserTabs{Tabs: listed})
}

func (i *instance) call(
	ctx context.Context,
	conversation webapiproto.ConversationID,
	method string,
	params json.RawMessage,
	port plugin.Port,
) (json.RawMessage, error) {
	result, err := i.runMethod(ctx, conversation, method, params, port)
	if err != nil {
		return nil, err
	}
	if err := port.Changed(ctx, plugin.ScopeConversation); err != nil {
		return nil, plugin.RequestError(err)
	}
	return result, nil
}

func (i *instance) runMethod(
	ctx context.Context,
	conversation webapiproto.ConversationID,
	method string,
	params json.RawMessage,
	port plugin.Port,
) (json.RawMessage, error) {
	var input browserproto.Input
	switch method {
	case "bind":
		args, err := DecodeBindTab(params)
		if err != nil {
			return nil, &plugin.ErrorUsage{Message: err.Error()}
		}
		return json.RawMessage("null"), i.bind(ctx, conversation, port, args.PanelTab)
	case "sync":
		if _, err := DecodeSyncTabs(params); err != nil {
			return nil, &plugin.ErrorUsage{Message: err.Error()}
		}
		return json.RawMessage("null"), i.syncTabs(ctx, conversation, port)
	case "navigate":
		args, err := DecodeNavigateTab(params)
		if err != nil {
			return nil, &plugin.ErrorUsage{Message: err.Error()}
		}
		return json.RawMessage("null"), navigate(ctx, port, args.Tab, args.URL)
	case "history":
		args, err := DecodeTabHistory(params)
		if err != nil {
			return nil, &plugin.ErrorUsage{Message: err.Error()}
		}
		tab, err := tabID(args.Tab)
		if err != nil {
			return nil, err
		}
		switch args.Action {
		case HistoryBack:
			input = &browserproto.BackInput{Tab: tab}
		case HistoryForward:
			input = &browserproto.ForwardInput{Tab: tab}
		case HistoryReload:
			input = &browserproto.ReloadInput{Tab: tab}
		}
	default:
		return nil, &plugin.ErrorFailed{Message: fmt.Sprintf("the browser has no method %s", method)}
	}
	if _, err := run(ctx, port, input, plugin.CallKindOperates); err != nil {
		return nil, refused(err)
	}
	return json.RawMessage("null"), nil
}

func run(
	ctx context.Context,
	port plugin.Port,
	input browserproto.Input,
	kind plugin.CallKind,
) (json.RawMessage, error) {
	args, err := contract.EncodeJSON(input)
	if err != nil {
		return nil, err
	}
	return port.PackageCall(ctx, operation(input.OperationName()), args, kind)
}

func unreadable(err error) error {
	return plugin.RequestError(
		&plugin.PortRefusalOperation{Stderr: "the browser answered what the plugin cannot read: " + err.Error()},
	)
}

func tabID(tab string) (browserproto.TabID, error) {
	id, err := browserproto.ParseTabID(tab)
	if err != nil {
		return "", &plugin.ErrorRefused{Reason: "tab_not_found", Message: "No such tab"}
	}
	return id, nil
}

func stopped(err error) bool {
	var refusal *plugin.PortRefusalHost
	return errors.As(err, &refusal) && refusal.Code == webapiproto.ErrorCodeHostStopped
}

func browserFailure(err error) (browserproto.FailureDocument, bool) {
	var refusal *plugin.PortRefusalOperation
	if !errors.As(err, &refusal) {
		return browserproto.FailureDocument{}, false
	}
	document, decodeErr := browserproto.DecodeFailureDocument([]byte(strings.TrimSpace(refusal.Stderr)))
	if decodeErr != nil {
		return browserproto.FailureDocument{}, false
	}
	return document, true
}

func tabMissing(err error) bool {
	document, ok := browserFailure(err)
	return ok && document.Error.Code == browserproto.BrowserErrorCodeTabNotFound
}

func refused(err error) error {
	if document, ok := browserFailure(err); ok {
		return &plugin.ErrorRefused{Reason: string(document.Error.Code), Message: document.Error.Message}
	}
	return plugin.RequestError(err)
}

func openTab(ctx context.Context, url string, port plugin.Port) (browserproto.BrowserTab, error) {
	result, err := run(ctx, port, &browserproto.OpenInput{URL: url}, plugin.CallKindStarts)
	if err != nil {
		return browserproto.BrowserTab{}, refused(err)
	}
	opened, err := browserproto.DecodeOpenResult(result)
	if err != nil {
		return browserproto.BrowserTab{}, unreadable(err)
	}
	title := ""
	if opened.Title != nil {
		title = *opened.Title
	}
	return browserproto.BrowserTab{
		ID:        opened.Tab,
		Title:     title,
		URL:       opened.URL,
		CreatedBy: &browserproto.BrowserCreatedByUser{},
	}, nil
}

func closeTab(ctx context.Context, tab string, port plugin.Port) error {
	id, err := browserproto.ParseTabID(tab)
	if err != nil {
		return nil
	}
	_, err = run(ctx, port, &browserproto.CloseInput{Tab: id}, plugin.CallKindOperates)
	if err != nil && !stopped(err) && !tabMissing(err) {
		return refused(err)
	}
	return nil
}

func navigate(ctx context.Context, port plugin.Port, tab, url string) error {
	id, err := tabID(tab)
	if err != nil {
		return err
	}
	_, err = run(ctx, port, &browserproto.GotoInput{Tab: id, URL: url}, plugin.CallKindOperates)
	if err != nil {
		return refused(err)
	}
	return nil
}
