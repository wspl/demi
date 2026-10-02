package page

import (
	"context"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// selectOptions matches native options in document order and shares readiness retries.
func selectOptions(ctx context.Context, tab *tabs.Tab, input browserop.SelectInput, refs *tabs.References, operation *cdp.Operation) (browserop.ActionResult, error) {
	result := browserop.ActionResult{Operation: "select"}
	candidates := 0
	if input.Value != nil {
		candidates++
	}
	if input.OptionLabel != nil {
		candidates++
	}
	if input.OptionIndex != nil {
		candidates++
	}
	if candidates != 1 || (input.Value != nil && len(*input.Value) == 0) || (input.OptionLabel != nil && len(*input.OptionLabel) == 0) || (input.OptionIndex != nil && len(*input.OptionIndex) == 0) {
		return result, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "select requires one nonempty value, option-label, or option-index list"}
	}
	type selectedOptions struct {
		Status  string                      `json:"status"`
		Options *[]browserop.SelectedOption `json:"options"`
	}
	var last error
	for {
		ready, err := ready(ctx, tab, input.BrowserTarget, refs, []string{"selectable", "enabled"}, operation, &last)
		if err != nil {
			return result, err
		}
		selected, err := callWithStates[selectedOptions](ctx, ready.element, scriptSelectOptions, input.Value, input.OptionLabel, input.OptionIndex, false)
		if err != nil {
			return result, err
		}
		if selected.Status == "ready" && selected.Options == nil {
			return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "select options result is missing options"}
		}
		if selected.Status == "ready" {
			operation.BeginInput()
			selected, err = callWithStates[selectedOptions](ctx, ready.element, scriptSelectOptions, input.Value, input.OptionLabel, input.OptionIndex, true)
			if err != nil {
				return result, err
			}
			if selected.Status == "ready" {
				if selected.Options == nil {
					return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "select options result is missing options"}
				}
				operation.CompleteInput()
				result.Target = &ready.named
				result.Result, err = cdp.Value(*selected.Options)
				return result, err
			}
			operation.InputNotDelivered()
		}
		switch selected.Status {
		case "disabled":
			condition := "enabled select option or control"
			last = &cdp.BrowserError{Kind: cdp.KindNotActionable, Details: browserop.ErrorDetails{Condition: &condition}}
		case "missing":
			last = &cdp.BrowserError{Kind: cdp.KindTargetNotFound}
		default:
			return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "unknown select options result"}
		}
		err = operation.Run(ctx, resampleWait)
		if err != nil {
			if cdp.IsDeadline(err) {
				err = cdp.WithDeadlineCause(err, last)
			}
			return result, err
		}
	}
}

// selectText selects rendered text or places the cursor at its requested edge.
func selectText(ctx context.Context, tab *tabs.Tab, input browserop.SelectTextInput, refs *tabs.References, operation *cdp.Operation) (browserop.ActionResult, error) {
	result := browserop.ActionResult{Operation: "select-text"}
	if input.Text == "" {
		return result, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "select-text requires nonempty text"}
	}
	type selection struct {
		Status string `json:"status"`
		Count  *uint  `json:"count"`
	}
	var last error
	for {
		ready, err := ready(ctx, tab, input.BrowserTarget, refs, []string{"visible"}, operation, &last)
		if err != nil {
			return result, err
		}
		operation.BeginInput()
		selected, err := decodeElement[selection](ctx, ready.element, scriptSelectText, false, input.Text, input.Cursor, input.Prefix, input.Suffix)
		if err != nil {
			return result, err
		}
		if selected.Status != "selected" {
			operation.InputNotDelivered()
		}
		switch selected.Status {
		case "selected":
			operation.CompleteInput()
			result.Target = &ready.named
			value := "selected"
			if input.Cursor != nil {
				value = string(*input.Cursor)
			}
			result.Result, err = cdp.Value(value)
			return result, err
		case "missing":
			condition := "requested text is rendered"
			last = &cdp.BrowserError{Kind: cdp.KindNotActionable, Details: browserop.ErrorDetails{Condition: &condition}}
			err = operation.Run(ctx, resampleWait)
			if err != nil {
				if cdp.IsDeadline(err) {
					err = cdp.WithDeadlineCause(err, last)
				}
				return result, err
			}
		case "ambiguous":
			if selected.Count == nil {
				return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "text selection result is missing count"}
			}
			return result, &cdp.BrowserError{Kind: cdp.KindAmbiguous, Count: *selected.Count}
		case "unsupported":
			condition := "text selection is unsupported on this input type"
			return result, &cdp.BrowserError{Kind: cdp.KindNotActionable, Details: browserop.ErrorDetails{Condition: &condition}}
		default:
			return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "unknown text selection result"}
		}
	}
}
