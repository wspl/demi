package page

import (
	"context"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// selectOptions matches native options in document order and shares readiness retries.
func selectOptions(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.SelectInput,
	refs *tabs.References,
	operation *cdp.Operation,
) (browserproto.ActionResult, error) {
	result := browserproto.ActionResult{Operation: "select"}
	if err := validateSelectedOptions(input); err != nil {
		return result, err
	}
	var last error
	for {
		ready, err := ready(ctx, tab, input.BrowserTarget, refs, []string{"selectable", "enabled"}, operation, &last)
		if err != nil {
			return result, err
		}
		selected, err := callWithStates[selectedOptions](
			ctx,
			ready.element,
			scriptSelectOptions,
			input.Value,
			input.OptionLabel,
			input.OptionIndex,
			false,
		)
		if err != nil {
			return result, err
		}
		if selected.Status == "ready" && selected.Options == nil {
			return result, &cdp.BrowserError{
				Kind:    cdp.KindInvalidResult,
				Message: "select options result is missing options",
			}
		}
		if selected.Status == "ready" {
			var completed bool
			selected, result, completed, err = commitSelectedOptions(ctx, input, operation, ready, result)
			if completed || err != nil {
				return result, err
			}
		}
		switch selected.Status {
		case "disabled":
			condition := "enabled select option or control"
			last = &cdp.BrowserError{
				Kind:    cdp.KindNotActionable,
				Details: browserproto.ErrorDetails{Condition: &condition},
			}
		case "missing":
			last = &cdp.BrowserError{Kind: cdp.KindTargetNotFound}
		default:
			return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "unknown select options result"}
		}
		if err := retrySelection(ctx, operation, last); err != nil {
			return result, err
		}
	}
}

// selectText selects rendered text or places the cursor at its requested edge.
func selectText(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.SelectTextInput,
	refs *tabs.References,
	operation *cdp.Operation,
) (browserproto.ActionResult, error) {
	result := browserproto.ActionResult{Operation: "select-text"}
	if input.Text == "" {
		return result, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "select-text requires nonempty text"}
	}
	var last error
	for {
		ready, err := ready(ctx, tab, input.BrowserTarget, refs, []string{"visible"}, operation, &last)
		if err != nil {
			return result, err
		}
		operation.BeginInput()
		selected, err := decodeElement[selection](
			ctx,
			ready.element,
			scriptSelectText,
			false,
			input.Text,
			input.Cursor,
			input.Prefix,
			input.Suffix,
		)
		if err != nil {
			return result, err
		}
		if selected.Status != "selected" {
			operation.InputNotDelivered()
		}
		switch selected.Status {
		case "selected":
			return selectedTextResult(input, operation, ready, result)
		case "missing":
			condition := "requested text is rendered"
			last = &cdp.BrowserError{
				Kind:    cdp.KindNotActionable,
				Details: browserproto.ErrorDetails{Condition: &condition},
			}
			err = retrySelection(ctx, operation, last)
			if err != nil {
				return result, err
			}
		default:
			return textSelectionFailure(selected, result)
		}
	}
}

func validateSelectedOptions(input browserproto.SelectInput) error {
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
	if candidates != 1 || (input.Value != nil && len(*input.Value) == 0) ||
		(input.OptionLabel != nil && len(*input.OptionLabel) == 0) ||
		(input.OptionIndex != nil && len(*input.OptionIndex) == 0) {
		return &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "select requires one nonempty value, option-label, or option-index list",
		}
	}

	return nil
}

func selectedTextResult(
	input browserproto.SelectTextInput,
	operation *cdp.Operation,
	ready readyElement,
	result browserproto.ActionResult,
) (browserproto.ActionResult, error) {
	operation.CompleteInput()
	result.Target = &ready.named
	value := "selected"
	if input.Cursor != nil {
		value = string(*input.Cursor)
	}
	encoded, err := cdp.Value(value)
	result.Result = encoded
	return result, err
}

type selectedOptions struct {
	// Status names the page algorithm outcome.
	Status string `json:"status"`
	// Options holds the matched native options when ready.
	Options *[]browserproto.SelectedOption `json:"options"`
}
type selection struct {
	// Status names the page algorithm outcome.
	Status string `json:"status"`
	// Count holds the number of ambiguous text matches.
	Count *uint `json:"count"`
}

func commitSelectedOptions(
	ctx context.Context,
	input browserproto.SelectInput,
	operation *cdp.Operation,
	ready readyElement,
	result browserproto.ActionResult,
) (selectedOptions, browserproto.ActionResult, bool, error) {
	operation.BeginInput()
	selected, err := callWithStates[selectedOptions](
		ctx,
		ready.element,
		scriptSelectOptions,
		input.Value,
		input.OptionLabel,
		input.OptionIndex,
		true,
	)
	if err != nil {
		return selected, result, true, err
	}
	if selected.Status == "ready" {
		if selected.Options == nil {
			return selected, result, true, &cdp.BrowserError{
				Kind:    cdp.KindInvalidResult,
				Message: "select options result is missing options",
			}
		}
		operation.CompleteInput()
		result.Target = &ready.named
		result.Result, err = cdp.Value(*selected.Options)
		return selected, result, true, err
	}
	operation.InputNotDelivered()
	return selected, result, false, nil
}

func textSelectionFailure(selected selection, result browserproto.ActionResult) (browserproto.ActionResult, error) {
	switch selected.Status {
	case "ambiguous":
		if selected.Count == nil {
			return result, &cdp.BrowserError{
				Kind:    cdp.KindInvalidResult,
				Message: "text selection result is missing count",
			}
		}
		return result, &cdp.BrowserError{Kind: cdp.KindAmbiguous, Count: *selected.Count}
	case "unsupported":
		condition := "text selection is unsupported on this input type"
		return result, &cdp.BrowserError{
			Kind:    cdp.KindNotActionable,
			Details: browserproto.ErrorDetails{Condition: &condition},
		}
	default:
		return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "unknown text selection result"}
	}
}

func retrySelection(ctx context.Context, operation *cdp.Operation, last error) error {
	err := operation.Run(ctx, resampleWait)
	if err != nil && cdp.IsDeadline(err) {
		err = cdp.WithDeadlineCause(err, last)
	}
	return err
}
