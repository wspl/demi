package browser

import (
	"context"
	"encoding/json"
	"time"

	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/page"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
)

type commandOutput struct {
	json json.RawMessage
	png  []byte
}

// resultOutput encodes a browser family's typed result for common rendering.
func resultOutput(result any, err error) (commandOutput, error) {
	if err != nil {
		return commandOutput{}, err
	}
	raw, err := cdp.Value(result)
	return commandOutput{json: raw}, err
}

func (s *service) execute(
	ctx, cancellation context.Context,
	browser *conversation,
	invocation *commandsdk.InvocationContext[commandproto.Invocation],
	command browserproto.Input,
	deadline time.Time,
) (commandOutput, error) {
	var start *starting
	_, user := invocation.Request.Context.Caller.(*commandproto.UserCaller)
	switch command.(type) {
	case *browserproto.OpenInput, *browserproto.ContentFetchInput:
		start = &starting{locale: invocation.Request.Context.Locale, invocation: invocation.Request.InvocationID}
	}
	environment, _, err := browser.running(ctx, start)
	if err != nil {
		return commandOutput{}, err
	}
	if environment == nil {
		if _, ok := command.(*browserproto.TabsInput); ok {
			return resultOutput(browserproto.TabsResult{Tabs: []browserproto.BrowserTab{}}, nil)
		}
		return commandOutput{}, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	switch input := command.(type) {
	case *browserproto.OpenInput:
		return open(ctx, environment, invocation, input, deadline)
	case *browserproto.ContentFetchInput:
		result, err := page.ContentFetch(ctx, invocation, environment, *input, deadline)
		select {
		case <-environment.Emptied():
			err = cdp.AfterCleanup(err, browser.retire(context.WithoutCancel(ctx), environment))
		default:
		}
		return resultOutput(result, err)
	case *browserproto.TabsInput:
		return listTabs(ctx, environment, input)
	}
	id, ok := command.TabID()
	if !ok {
		return commandOutput{}, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	tab, err := environment.Tab(ctx, id, command.Timeout())
	if err != nil {
		return commandOutput{}, err
	}
	if user {
		switch command.(type) {
		case *browserproto.GotoInput, *browserproto.ReloadInput, *browserproto.BackInput, *browserproto.ForwardInput:
			operation := tab.Operation(ctx, deadline)
			defer operation.Close()
			return resultOutput(tab.Steer(ctx, command, operation))
		}
	}
	return executeTab(ctx, cancellation, browser, environment, tab, invocation, command, deadline)
}

func executeTab(
	ctx, cancellation context.Context,
	browser *conversation,
	environment *tabs.Environment,
	tab *tabs.Tab,
	invocation *commandsdk.InvocationContext[commandproto.Invocation],
	command browserproto.Input,
	deadline time.Time,
) (commandOutput, error) {
	switch input := command.(type) {
	case *browserproto.CloseInput:
		return closeTab(ctx, browser, environment, tab, command.Timeout())
	case *browserproto.CapabilitiesInput:
		return capabilities(ctx, tab, deadline)
	case *browserproto.ScreenshotInput:
		return screenshot(ctx, tab, invocation, input)
	case *browserproto.UploadInput:
		return resultOutput(page.Upload(ctx, invocation, tab, *input, deadline))
	case *browserproto.DownloadInput:
		return resultOutput(page.Download(ctx, invocation, environment, tab, *input, deadline))
	case *browserproto.ClipboardReadInput:
		return resultOutput(page.ClipboardRead(ctx, invocation, environment, tab, *input, deadline))
	case *browserproto.ClipboardWriteInput:
		return resultOutput(page.ClipboardWrite(ctx, invocation, environment, tab, *input, deadline))
	case *browserproto.AssetsListInput:
		return resultOutput(page.AssetsList(ctx, invocation, tab, *input, deadline))
	case *browserproto.AssetsExportInput:
		return resultOutput(page.AssetsExport(ctx, invocation, tab, *input, deadline))
	case *browserproto.CDPTargetsInput,
		*browserproto.CDPDetachInput,
		*browserproto.CDPSendInput,
		*browserproto.CDPEventsInput:
		return executeDebugging(ctx, cancellation, tab, invocation, command, deadline)
	case *browserproto.WebMCPListInput, *browserproto.WebMCPCallInput:
		return webMCP(ctx, tab, command, deadline)
	case *browserproto.ProbeInput:
		if input.Output != nil {
			return probe(ctx, tab, invocation, input, deadline)
		}
	}
	return executePage(ctx, tab, invocation, command, deadline)
}

func executePage(
	ctx context.Context,
	tab *tabs.Tab,
	invocation *commandsdk.InvocationContext[commandproto.Invocation],
	command browserproto.Input,
	deadline time.Time,
) (commandOutput, error) {
	raw, err := page.Command(ctx, tab, command, deadline)
	if err != nil {
		return commandOutput{}, err
	}
	if input, ok := command.(*browserproto.ContentReadInput); ok && input.Output != nil {
		result, err := browserproto.DecodeContentReadResult(raw)
		if err != nil {
			return commandOutput{}, err
		}
		inline, ok := result.(*browserproto.ContentReadResultInline)
		if !ok {
			return commandOutput{}, &cdp.BrowserError{
				Kind:    cdp.KindInvalidResult,
				Message: "content export did not return text",
			}
		}
		path, err := cdp.SaveWithOverwrite(
			ctx,
			invocation.Request.Cwd,
			*input.Output,
			[]byte(inline.Content),
			input.Overwrite != nil && *input.Overwrite,
		)
		return resultOutput(
			&browserproto.ContentReadResultFile{
				URL:    inline.URL,
				Title:  inline.Title,
				Format: inline.Format,
				Path:   path,
			},
			err,
		)
	}
	return commandOutput{json: raw}, nil
}

func open(
	ctx context.Context,
	environment *tabs.Environment,
	invocation *commandsdk.InvocationContext[commandproto.Invocation],
	input *browserproto.OpenInput,
	deadline time.Time,
) (commandOutput, error) {
	if _, user := invocation.Request.Context.Caller.(*commandproto.UserCaller); user {
		var url *string
		if input.URL != "about:blank" {
			url = &input.URL
		}
		tab, err := environment.OpenUser(ctx, url, deadline)
		if err != nil {
			return commandOutput{}, err
		}
		return resultOutput(browserproto.OpenResult{Tab: tab.ID(), URL: input.URL}, nil)
	}
	caller, err := cdp.Agent(invocation)
	if err != nil {
		return commandOutput{}, err
	}
	load := browserproto.LoadDOMContentLoaded
	if input.Load != nil {
		load = *input.Load
	}
	tab, url, err := environment.OpenFor(ctx, input.URL, caller, load, deadline)
	if err != nil {
		return commandOutput{}, err
	}
	result := browserproto.OpenResult{Tab: tab.ID(), URL: url}
	metadata, err := page.Metadata(ctx, tab, time.Until(deadline))
	if err == nil && metadata.URL == url {
		result.Title = &metadata.Title
		result.Viewport = &metadata.Viewport
	}
	return resultOutput(result, nil)
}

func capabilities(ctx context.Context, tab *tabs.Tab, deadline time.Time) (commandOutput, error) {
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return commandOutput{}, operation.Failure(&cdp.BrowserError{Kind: cdp.KindBusy}, string(tab.ID()), nil)
	}
	defer checkout.Release()
	if tab.Dialog().IsOpen() {
		return commandOutput{}, operation.Failure(&cdp.BrowserError{Kind: cdp.KindDialogBlocked}, string(tab.ID()), nil)
	}
	var capabilities []browserproto.Capability
	err := operation.Run(ctx, func(work context.Context) error {
		var err error
		capabilities, err = page.Capabilities(work, tab)
		if err != nil {
			return err
		}
		families, err := cdp.Capabilities(work, tab.Page())
		capabilities = append(capabilities, families...)
		return err
	})
	if err != nil {
		return commandOutput{}, operation.Failure(err, string(tab.ID()), nil)
	}
	return resultOutput(browserproto.CapabilitiesResult{Capabilities: capabilities}, nil)
}

func webMCP(
	ctx context.Context,
	tab *tabs.Tab,
	command browserproto.Operation,
	deadline time.Time,
) (commandOutput, error) {
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return commandOutput{}, &cdp.BrowserError{Kind: cdp.KindBusy}
	}
	defer checkout.Release()
	raw, err := cdp.ExecuteWebMCP(ctx, operation, tab.Page(), &checkout.Session().WebMCP, tab.ID(), command)
	return commandOutput{json: raw}, err
}

func listTabs(
	ctx context.Context,
	environment *tabs.Environment,
	input *browserproto.TabsInput,
) (commandOutput, error) {
	listing, err := environment.Listed(ctx, input.Timeout())
	if err != nil {
		return commandOutput{}, err
	}
	offset, limit := uint(0), uint(browserproto.DefaultNodes)
	if input.Offset != nil {
		offset = *input.Offset
	}
	if input.Limit != nil {
		limit = *input.Limit
	}
	rows := []browserproto.BrowserTab{}
	offset = min(offset, uint(len(listing.Tabs)))
	end := offset + min(limit, uint(len(listing.Tabs))-offset)
	for _, row := range listing.Tabs[offset:end] {
		rows = append(
			rows,
			browserproto.BrowserTab{
				ID:        row.Tab.ID(),
				URL:       row.URL,
				Title:     row.Title,
				CreatedBy: row.Tab.CreatedBy(),
				Loading:   row.Tab.Loading(),
			},
		)
	}
	return resultOutput(browserproto.TabsResult{Tabs: rows, Truncated: end < uint(len(listing.Tabs))}, nil)
}

func closeTab(
	ctx context.Context,
	browser *conversation,
	environment *tabs.Environment,
	tab *tabs.Tab,
	timeout time.Duration,
) (commandOutput, error) {
	emptied, err := tab.CloseRequest(ctx, timeout)
	if err != nil {
		return commandOutput{}, err
	}
	if emptied {
		err = browser.retire(context.WithoutCancel(ctx), environment)
	} else {
		select {
		case <-environment.Emptied():
			err = browser.retire(context.WithoutCancel(ctx), environment)
		default:
		}
	}
	return resultOutput(browserproto.CloseResult{Closed: tab.ID()}, err)
}

func executeDebugging(
	ctx, cancellation context.Context,
	tab *tabs.Tab,
	invocation *commandsdk.InvocationContext[commandproto.Invocation],
	command browserproto.Input,
	deadline time.Time,
) (commandOutput, error) {
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		return commandOutput{}, &cdp.BrowserError{Kind: cdp.KindBusy}
	}
	defer checkout.Release()
	caller, err := cdp.Agent(invocation)
	if err != nil {
		return commandOutput{}, err
	}
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	// CDP event expiry returns an empty page; only invocation cancellation
	// detaches the subscription. The operation retains the absolute deadline.
	raw, err := cdp.ExecuteCommand(cancellation, operation, tab.Debug(), caller, tab.ID(), command)
	if cancellation.Err() != nil {
		err = cdp.AfterCleanup(err, tab.Debug().Detach(context.WithoutCancel(cancellation), caller))
	}
	return commandOutput{json: raw}, err
}
