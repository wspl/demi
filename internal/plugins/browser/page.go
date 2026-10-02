package browser

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

func tabs(ctx context.Context, port plugin.Port) (json.RawMessage, error) {
	result, err := run(ctx, port, &browserop.TabsInput{}, plugin.CallKindLooks)
	if stopped(err) {
		return contract.EncodeJSON(browserTabs{Tabs: []browserop.BrowserTab{}})
	}
	if err != nil {
		return nil, refused(err)
	}
	listed, err := browserop.DecodeTabsResult(result)
	if err != nil {
		return nil, unreadable(err)
	}
	return contract.EncodeJSON(browserTabs{Tabs: listed.Tabs})
}

func call(ctx context.Context, method string, params json.RawMessage, port plugin.Port) (json.RawMessage, error) {
	result, err := runMethod(ctx, method, params, port)
	if err != nil {
		return nil, err
	}
	if err := port.Changed(ctx, plugin.ScopeConversation); err != nil {
		return nil, plugin.RequestError(err)
	}
	return result, nil
}

func runMethod(ctx context.Context, method string, params json.RawMessage, port plugin.Port) (json.RawMessage, error) {
	var input browserop.Input
	switch method {
	case "open":
		args, err := DecodeOpenTab(params)
		if err != nil {
			return nil, &plugin.ErrorUsage{Message: err.Error()}
		}
		url := "about:blank"
		if args.URL != nil {
			url = *args.URL
		}
		result, err := run(ctx, port, &browserop.OpenInput{URL: url}, plugin.CallKindStarts)
		if err != nil {
			return nil, refused(err)
		}
		opened, err := browserop.DecodeOpenResult(result)
		if err != nil {
			return nil, unreadable(err)
		}
		title := ""
		if opened.Title != nil {
			title = *opened.Title
		}
		return contract.EncodeJSON(OpenedTab{Tab: browserop.BrowserTab{ID: opened.Tab, Title: title, URL: opened.URL, CreatedBy: &browserop.BrowserCreatedByUser{}}})
	case "close":
		args, err := DecodeCloseTab(params)
		if err != nil {
			return nil, &plugin.ErrorUsage{Message: err.Error()}
		}
		tab, err := browserop.ParseTabID(args.Tab)
		if err != nil {
			return json.RawMessage("null"), nil
		}
		_, err = run(ctx, port, &browserop.CloseInput{Tab: tab}, plugin.CallKindOperates)
		if err != nil && !stopped(err) && !tabMissing(err) {
			return nil, refused(err)
		}
		return json.RawMessage("null"), nil
	case "navigate":
		args, err := DecodeNavigateTab(params)
		if err != nil {
			return nil, &plugin.ErrorUsage{Message: err.Error()}
		}
		tab, err := tabID(args.Tab)
		if err != nil {
			return nil, err
		}
		input = &browserop.GotoInput{Tab: tab, URL: args.URL}
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
			input = &browserop.BackInput{Tab: tab}
		case HistoryForward:
			input = &browserop.ForwardInput{Tab: tab}
		case HistoryReload:
			input = &browserop.ReloadInput{Tab: tab}
		}
	default:
		return nil, &plugin.ErrorFailed{Message: fmt.Sprintf("the browser has no method %s", method)}
	}
	if _, err := run(ctx, port, input, plugin.CallKindOperates); err != nil {
		return nil, refused(err)
	}
	return json.RawMessage("null"), nil
}

func run(ctx context.Context, port plugin.Port, input browserop.Input, kind plugin.CallKind) (json.RawMessage, error) {
	args, err := contract.EncodeJSON(input)
	if err != nil {
		return nil, err
	}
	return port.PackageCall(ctx, operation(input.OperationName()), args, kind)
}

func unreadable(err error) error {
	return plugin.RequestError(&plugin.PortRefusalOperation{Stderr: "the browser answered what the plugin cannot read: " + err.Error()})
}

func tabID(tab string) (browserop.TabID, error) {
	id, err := browserop.ParseTabID(tab)
	if err != nil {
		return "", &plugin.ErrorRefused{Reason: "tab_not_found", Message: "No such tab"}
	}
	return id, nil
}

func stopped(err error) bool {
	var refusal *plugin.PortRefusalHost
	return errors.As(err, &refusal) && refusal.Code == webapi.ErrorCodeHostStopped
}

func browserFailure(err error) *browserop.FailureDocument {
	var refusal *plugin.PortRefusalOperation
	if !errors.As(err, &refusal) {
		return nil
	}
	document, decodeErr := browserop.DecodeFailureDocument([]byte(strings.TrimSpace(refusal.Stderr)))
	if decodeErr != nil {
		return nil
	}
	return &document
}

func tabMissing(err error) bool {
	document := browserFailure(err)
	return document != nil && document.Error.Code == browserop.BrowserErrorCodeTabNotFound
}

func refused(err error) error {
	if document := browserFailure(err); document != nil {
		return &plugin.ErrorRefused{Reason: string(document.Error.Code), Message: document.Error.Message}
	}
	return plugin.RequestError(err)
}
