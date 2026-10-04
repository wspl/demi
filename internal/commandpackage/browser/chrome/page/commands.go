package page

import (
	"context"
	"encoding/json"
	"errors"
	"slices"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
)

// Execute decodes and validates a tab command, then uses the same admission as the service.
// The returned JSON is encoded through the result contract, which fixes its wire bytes.
func Execute(ctx context.Context, tab *tabs.Tab, name string, args json.RawMessage) (json.RawMessage, error) {
	command, err := browserproto.ParseOperation(name, args)
	if err != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: err.Error(), Cause: err}
	}
	input, ok := command.(browserproto.Input)
	if !ok {
		return nil, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	if id, targeted := input.TabID(); !targeted || id != tab.ID() {
		return nil, &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	switch command.(type) {
	case *browserproto.OpenInput,
		*browserproto.TabsInput,
		*browserproto.CloseInput,
		*browserproto.ScreenshotInput,
		*browserproto.CapabilitiesInput:
		return nil, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "this operation requires the browser conversation controller",
		}
	}
	return Command(ctx, tab, command, time.Now().Add(input.Timeout()))
}

// Command executes a parsed tab command under one admission and absolute deadline.
// Results are encoded through their browserop contract; no second result union is declared.
func Command(
	ctx context.Context,
	tab *tabs.Tab,
	command browserproto.Operation,
	deadline time.Time,
) (json.RawMessage, error) {
	checkout := tab.Gate().TryCheckout()
	if checkout == nil {
		operation := tab.Operation(ctx, deadline)
		defer operation.Close()
		return nil, operation.Failure(&cdp.BrowserError{Kind: cdp.KindBusy}, string(tab.ID()), nil)
	}
	defer checkout.Release()
	return CommandAdmitted(ctx, tab, command, deadline, &checkout.Session().References)
}

// CommandAdmitted keeps one browser-tab admission through a command and its attached artifact capture.
// The caller holds the tab checkout and passes its References until this call returns.
func CommandAdmitted(
	ctx context.Context,
	tab *tabs.Tab,
	command browserproto.Operation,
	deadline time.Time,
	references *tabs.References,
) (json.RawMessage, error) {
	operation := tab.Operation(ctx, deadline)
	defer operation.Close()
	input, ok := command.(browserproto.Input)
	if !ok {
		return nil, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "this operation requires the browser conversation controller",
		}
	}
	dialogCommand, err := admitCommand(tab, command, operation)
	if err != nil {
		return nil, err
	}
	navigation, err := commandNavigation(ctx, tab, input, command, operation)
	if err != nil {
		return nil, err
	}
	if navigation != nil {
		defer navigation.Close()
	}
	currentURL, err := currentCommandURL(ctx, operation, tab)
	if err != nil {
		return nil, operation.Failure(err, string(tab.ID()), nil)
	}
	var before []browserproto.TabID
	var observedBefore bool
	if opensTabs(command) {
		err := operation.Run(ctx, func(work context.Context) error {
			var err error
			before, err = tab.Opened(work)
			return err
		})
		observedBefore = err == nil // Popup metadata cannot prevent native input.
	}
	var result any
	// Input branches retain their cancellation cleanup and readiness failure.
	// An outer Run would replace a resolved not_actionable failure with timeout.
	result, err = dispatch(operation.Context(), tab, command, references, operation, navigation)
	if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		original := err
		err = operation.Run(ctx, func(context.Context) error { return original })
	}

	if !dialogCommand {
		err = cdp.AfterCleanup(err, releaseObjects(ctx, tab))
	}
	if err == nil && observedBefore {
		result = includeOpenedTabs(operation, tab, result, before)
	}
	if navigation != nil && navigation.DocumentChanged() {
		references.Invalidate()
	}
	if err != nil {
		return nil, operation.Failure(err, string(tab.ID()), currentURL)
	}
	return cdp.Value(result)
}

// Metadata reads current tab information without acquiring the agent command gate.
func Metadata(ctx context.Context, tab *tabs.Tab, timeout time.Duration) (browserproto.InfoResult, error) {
	operation := tab.Operation(ctx, time.Now().Add(timeout))
	defer operation.Close()
	var result browserproto.InfoResult
	err := operation.Run(ctx, func(work context.Context) error {
		url, title, err := targetInfo(work, tab)
		if err != nil {
			return err
		}
		result = browserproto.InfoResult{Tab: tab.ID(), URL: url, Title: title, Viewport: tab.Viewport()}
		if dialog := tab.Dialog().Open(); dialog != nil {
			result.Dialog = &browserproto.Dialog{Type: tabs.DialogType(dialog.Type), Message: dialog.Message}
		}
		return nil
	})
	return result, err
}

// targetInfo reads the current title and URL from Chrome's target registry.
func targetInfo(ctx context.Context, tab *tabs.Tab) (string, string, error) {
	info, err := target.GetTargetInfo().WithTargetID(tab.TargetID()).Do(protocol.WithExecutor(ctx, tab.Page()))
	if err != nil {
		return "", "", err
	}
	return info.URL, info.Title, nil
}

// opensTabs identifies pointer and form actions whose result reports new tabs.
func opensTabs(command browserproto.Operation) bool {
	switch command.(type) {
	case *browserproto.ClickInput,
		*browserproto.MoveInput,
		*browserproto.DragInput,
		*browserproto.ScrollInput,
		*browserproto.FillInput,
		*browserproto.TypeInput,
		*browserproto.KeyInput,
		*browserproto.CheckInput,
		*browserproto.SelectInput,
		*browserproto.SelectTextInput:
		return true
	}
	return false
}

// completedNavigation never turns completed navigation into a metadata failure.
func completedNavigation(ctx context.Context, tab *tabs.Tab, url string) browserproto.NavigationResult {
	result := browserproto.NavigationResult{Tab: tab.ID(), URL: url}
	bounded, cancel := context.WithTimeout(ctx, cdp.ControlTimeout)
	defer cancel()
	current, title, err := targetInfo(bounded, tab)
	if err == nil && current == url {
		result.Title = &title
	}
	return result
}

// actionResult observes navigation only after input has completed.
func actionResult(
	ctx context.Context,
	tab *tabs.Tab,
	operation *cdp.Operation,
	name string,
	wait *string,
	navigation *tabs.NavigationObservation,
) (browserproto.ActionResult, error) {
	result := browserproto.ActionResult{Operation: name, Result: json.RawMessage(`"completed"`)}
	if navigation != nil {
		var err error
		if wait != nil {
			err = navigation.WaitURL(ctx, tab.Page(), *wait)
		} else if name == "click" {
			err = navigation.WaitLoad(ctx, "domcontentloaded", true, operation)
		}
		url := navigation.URL()
		if err != nil {
			return result, operation.Failure(err, string(tab.ID()), &url)
		}
		result.URL = &url
	}
	return result, nil
}

func includeOpenedTabs(operation *cdp.Operation, tab *tabs.Tab, result any, before []browserproto.TabID) any {
	if action, ok := result.(browserproto.ActionResult); ok {
		bounded, cancel := context.WithTimeout(operation.Context(), cdp.ControlTimeout)
		after, popupErr := tab.Popups(bounded)
		cancel()
		if popupErr == nil {
			opened := slices.DeleteFunc(after, func(id browserproto.TabID) bool { return slices.Contains(before, id) })
			if len(opened) > 0 {
				action.OpenedTabs = &opened
			}
		}
		result = action
	}
	return result
}

func currentCommandURL(ctx context.Context, operation *cdp.Operation, tab *tabs.Tab) (*string, error) {
	var currentURL *string
	err := operation.Run(ctx, func(work context.Context) error {
		url, _, err := targetInfo(work, tab)
		if err == nil {
			currentURL = &url
		}
		return err
	})
	if err != nil {
		return nil, err
	}

	return currentURL, nil
}

func admitCommand(tab *tabs.Tab, command browserproto.Operation, operation *cdp.Operation) (bool, error) {
	dialogCommand := false
	switch command.(type) {
	case *browserproto.DialogInspectInput, *browserproto.DialogAcceptInput, *browserproto.DialogDismissInput:
		dialogCommand = true
	}
	if !dialogCommand && tab.Dialog().IsOpen() {
		return false, operation.Failure(&cdp.BrowserError{Kind: cdp.KindDialogBlocked}, string(tab.ID()), nil)
	}

	return dialogCommand, nil
}

func commandNavigation(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.Input,
	command browserproto.Operation,
	operation *cdp.Operation,
) (*tabs.NavigationObservation, error) {
	var navigation *tabs.NavigationObservation
	_, click := command.(*browserproto.ClickInput)
	if _, wait := input.WaitURLPattern(); wait || click {
		err := operation.Run(ctx, func(work context.Context) error {
			var err error
			navigation, err = tab.ObserveNavigation(work)
			return err
		})
		if err != nil {
			return nil, operation.Failure(err, string(tab.ID()), nil)
		}
	}
	return navigation, nil
}
