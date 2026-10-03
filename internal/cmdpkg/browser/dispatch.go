package browser

import (
	"context"
	"encoding/json"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
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
	invocation *cmdsdk.InvocationContext[commandwire.Invocation],
	command browserop.Input,
	deadline time.Time,
) (commandOutput, error) {
	var start *starting
	_, user := invocation.Request.Context.Caller.(*commandwire.UserCaller)
	switch command.(type) {
	case *browserop.OpenInput, *browserop.ContentFetchInput:
		start = &starting{locale: invocation.Request.Context.Locale, invocation: invocation.Request.InvocationID}
	}
	environment, _, err := browser.running(ctx, start)
	if err != nil {
		return commandOutput{}, err
	}
	if environment == nil {
		if _, ok := command.(*browserop.TabsInput); ok {
			return resultOutput(browserop.TabsResult{Tabs: []browserop.BrowserTab{}}, nil)
		}
		return commandOutput{}, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	switch input := command.(type) {
	case *browserop.OpenInput:
		return open(ctx, environment, invocation, input, deadline)
	case *browserop.ContentFetchInput:
		result, err := page.ContentFetch(ctx, invocation, environment, *input, deadline)
		select {
		case <-environment.Emptied():
			err = cdp.AfterCleanup(err, browser.retire(context.WithoutCancel(ctx), environment))
		default:
		}
		return resultOutput(result, err)
	case *browserop.TabsInput:
		return listTabs(ctx, environment, input)
	}
	id := command.TabID()
	if id == nil {
		return commandOutput{}, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	tab, err := environment.Tab(ctx, *id, command.Timeout())
	if err != nil {
		return commandOutput{}, err
	}
	if user {
		switch command.(type) {
		case *browserop.GotoInput, *browserop.ReloadInput, *browserop.BackInput, *browserop.ForwardInput:
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
	invocation *cmdsdk.InvocationContext[commandwire.Invocation],
	command browserop.Input,
	deadline time.Time,
) (commandOutput, error) {
	switch input := command.(type) {
	case *browserop.CloseInput:
		return closeTab(ctx, browser, environment, tab, command.Timeout())
	case *browserop.CapabilitiesInput:
		return capabilities(ctx, tab, deadline)
	case *browserop.ScreenshotInput:
		return screenshot(ctx, tab, invocation, input)
	case *browserop.UploadInput:
		return resultOutput(page.Upload(ctx, invocation, tab, *input, deadline))
	case *browserop.DownloadInput:
		return resultOutput(page.Download(ctx, invocation, environment, tab, *input, deadline))
	case *browserop.ClipboardReadInput:
		return resultOutput(page.ClipboardRead(ctx, invocation, environment, tab, *input, deadline))
	case *browserop.ClipboardWriteInput:
		return resultOutput(page.ClipboardWrite(ctx, invocation, environment, tab, *input, deadline))
	case *browserop.AssetsListInput:
		return resultOutput(page.AssetsList(ctx, invocation, tab, *input, deadline))
	case *browserop.AssetsExportInput:
		return resultOutput(page.AssetsExport(ctx, invocation, tab, *input, deadline))
	case *browserop.CdpTargetsInput, *browserop.CdpDetachInput, *browserop.CdpSendInput, *browserop.CdpEventsInput:
		return executeDebugging(ctx, cancellation, tab, invocation, command, deadline)
	case *browserop.WebmcpListInput, *browserop.WebmcpCallInput:
		return webMCP(ctx, tab, command, deadline)
	case *browserop.ProbeInput:
		if input.Output != nil {
			return probe(ctx, tab, invocation, input, deadline)
		}
	}
	return executePage(ctx, tab, invocation, command, deadline)
}

func executePage(
	ctx context.Context,
	tab *tabs.Tab,
	invocation *cmdsdk.InvocationContext[commandwire.Invocation],
	command browserop.Input,
	deadline time.Time,
) (commandOutput, error) {
	raw, err := page.Command(ctx, tab, command, deadline)
	if err != nil {
		return commandOutput{}, err
	}
	if input, ok := command.(*browserop.ContentReadInput); ok && input.Output != nil {
		result, err := browserop.DecodeContentReadResult(raw)
		if err != nil {
			return commandOutput{}, err
		}
		inline, ok := result.(*browserop.ContentReadResultInline)
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
			&browserop.ContentReadResultFile{URL: inline.URL, Title: inline.Title, Format: inline.Format, Path: path},
			err,
		)
	}
	return commandOutput{json: raw}, nil
}

func open(
	ctx context.Context,
	environment *tabs.Environment,
	invocation *cmdsdk.InvocationContext[commandwire.Invocation],
	input *browserop.OpenInput,
	deadline time.Time,
) (commandOutput, error) {
	if _, user := invocation.Request.Context.Caller.(*commandwire.UserCaller); user {
		var url *string
		if input.URL != "about:blank" {
			url = &input.URL
		}
		tab, err := environment.OpenUser(ctx, url, deadline)
		if err != nil {
			return commandOutput{}, err
		}
		return resultOutput(browserop.OpenResult{Tab: tab.ID(), URL: input.URL}, nil)
	}
	caller, err := cdp.Agent(invocation)
	if err != nil {
		return commandOutput{}, err
	}
	load := browserop.LoadDomContentLoaded
	if input.Load != nil {
		load = *input.Load
	}
	tab, url, err := environment.OpenFor(ctx, input.URL, caller, load, deadline)
	if err != nil {
		return commandOutput{}, err
	}
	result := browserop.OpenResult{Tab: tab.ID(), URL: url}
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
	var capabilities []browserop.Capability
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
	return resultOutput(browserop.CapabilitiesResult{Capabilities: capabilities}, nil)
}

func webMCP(
	ctx context.Context,
	tab *tabs.Tab,
	command browserop.Operation,
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

func listTabs(ctx context.Context, environment *tabs.Environment, input *browserop.TabsInput) (commandOutput, error) {
	listing, err := environment.Listed(ctx, input.Timeout())
	if err != nil {
		return commandOutput{}, err
	}
	offset, limit := uint(0), uint(browserop.DefaultNodes)
	if input.Offset != nil {
		offset = *input.Offset
	}
	if input.Limit != nil {
		limit = *input.Limit
	}
	rows := []browserop.BrowserTab{}
	offset = min(offset, uint(len(listing.Tabs)))
	end := offset + min(limit, uint(len(listing.Tabs))-offset)
	for _, row := range listing.Tabs[offset:end] {
		rows = append(
			rows,
			browserop.BrowserTab{ID: row.Tab.ID(), URL: row.URL, Title: row.Title, CreatedBy: row.Tab.CreatedBy()},
		)
	}
	return resultOutput(browserop.TabsResult{Tabs: rows, Truncated: end < uint(len(listing.Tabs))}, nil)
}

func closeTab(
	ctx context.Context,
	browser *conversation,
	environment *tabs.Environment,
	tab *tabs.Tab,
	timeout time.Duration,
) (commandOutput, error) {
	closed, err := tab.CloseRequest(ctx, timeout)
	if err != nil {
		return commandOutput{}, err
	}
	if closed == tabs.ClosedEnvironment {
		err = browser.retire(context.WithoutCancel(ctx), environment)
	} else {
		select {
		case <-environment.Emptied():
			err = browser.retire(context.WithoutCancel(ctx), environment)
		default:
		}
	}
	return resultOutput(browserop.CloseResult{Closed: tab.ID()}, err)
}

func executeDebugging(
	ctx, cancellation context.Context,
	tab *tabs.Tab,
	invocation *cmdsdk.InvocationContext[commandwire.Invocation],
	command browserop.Input,
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
