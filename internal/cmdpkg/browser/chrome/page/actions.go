package page

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// Info reads current tab information.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Info(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.InfoInput,
	deadline time.Time,
) (browserop.InfoResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.InfoResult
		return zero, err
	}
	return browserop.DecodeInfoResult(raw)
}

// Goto navigates to the requested URL.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Goto(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.GotoInput,
	deadline time.Time,
) (browserop.NavigationResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.NavigationResult
		return zero, err
	}
	return browserop.DecodeNavigationResult(raw)
}

// Back navigates to the previous history entry.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Back(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.BackInput,
	deadline time.Time,
) (browserop.NavigationResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.NavigationResult
		return zero, err
	}
	return browserop.DecodeNavigationResult(raw)
}

// Forward navigates to the next history entry.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Forward(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.ForwardInput,
	deadline time.Time,
) (browserop.NavigationResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.NavigationResult
		return zero, err
	}
	return browserop.DecodeNavigationResult(raw)
}

// Reload reloads the current document.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Reload(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.ReloadInput,
	deadline time.Time,
) (browserop.NavigationResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.NavigationResult
		return zero, err
	}
	return browserop.DecodeNavigationResult(raw)
}

// History reads the requested history page.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func History(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.HistoryInput,
	deadline time.Time,
) (browserop.HistoryResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.HistoryResult
		return zero, err
	}
	return browserop.DecodeHistoryResult(raw)
}

// Inspect observes the accessibility or DOM tree.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Inspect(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.InspectInput,
	deadline time.Time,
) (browserop.InspectResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.InspectResult
		return zero, err
	}
	return browserop.DecodeInspectResult(raw)
}

// Find resolves a locator or query and describes its matches.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Find(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.FindInput,
	deadline time.Time,
) (browserop.FindResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.FindResult
		return zero, err
	}
	return browserop.DecodeFindResult(raw)
}

// Read reads element properties while protecting password values.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Read(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.ReadInput,
	deadline time.Time,
) (browserop.ReadResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ReadResult
		return zero, err
	}
	return browserop.DecodeReadResult(raw)
}

// Probe describes the elements at viewport coordinates.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Probe(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.ProbeInput,
	deadline time.Time,
) (browserop.ProbeResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ProbeResult
		return zero, err
	}
	return browserop.DecodeProbeResult(raw)
}

// Click delivers a native click and observes its associated navigation.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Click(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.ClickInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// Move moves the pointer to an element or viewport coordinates.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Move(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.MoveInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// Drag delivers a native pointer drag.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Drag(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.DragInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// Scroll delivers a native wheel event.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Scroll(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.ScrollInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// Fill replaces the value of a fillable element.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Fill(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.FillInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// Type types text into a target or the focused element.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Type(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.TypeInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// Key delivers a validated key combination.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Key(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.KeyInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// Check sets and verifies the requested checked state.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Check(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.CheckInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// Select selects native options by value, label, or index.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Select(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.SelectInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// SelectText selects text in an element.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func SelectText(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.SelectTextInput,
	deadline time.Time,
) (browserop.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ActionResult
		return zero, err
	}
	return browserop.DecodeActionResult(raw)
}

// Wait waits for a URL, load, or element condition.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Wait(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.WaitInput,
	deadline time.Time,
) (browserop.WaitResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.WaitResult
		return zero, err
	}
	return browserop.DecodeWaitResult(raw)
}

// Eval evaluates a read-only expression with optional element targets.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Eval(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.EvalInput,
	deadline time.Time,
) (browserop.EvalResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.EvalResult
		return zero, err
	}
	return browserop.DecodeEvalResult(raw)
}

// Logs reads the tab console using its independent cursor.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Logs(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.LogsInput,
	deadline time.Time,
) (browserop.LogsResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.LogsResult
		return zero, err
	}
	return browserop.DecodeLogsResult(raw)
}

// ViewportSet sets the custom viewport.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func ViewportSet(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.ViewportSetInput,
	deadline time.Time,
) (browserop.ViewportResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ViewportResult
		return zero, err
	}
	return browserop.DecodeViewportResult(raw)
}

// ViewportReset restores the live-view viewport.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func ViewportReset(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.ViewportResetInput,
	deadline time.Time,
) (browserop.ViewportResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ViewportResult
		return zero, err
	}
	return browserop.DecodeViewportResult(raw)
}

// DialogInspect reads the current dialog.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func DialogInspect(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.DialogInspectInput,
	deadline time.Time,
) (browserop.DialogInspectResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.DialogInspectResult
		return zero, err
	}
	return browserop.DecodeDialogInspectResult(raw)
}

// DialogAccept accepts the observed dialog.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func DialogAccept(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.DialogAcceptInput,
	deadline time.Time,
) (browserop.DialogResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.DialogResult
		return zero, err
	}
	return browserop.DecodeDialogResult(raw)
}

// DialogDismiss dismisses the observed dialog.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func DialogDismiss(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.DialogDismissInput,
	deadline time.Time,
) (browserop.DialogResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.DialogResult
		return zero, err
	}
	return browserop.DecodeDialogResult(raw)
}

// ContentRead reads document content in the requested format.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func ContentRead(
	ctx context.Context,
	tab *tabs.Tab,
	input browserop.ContentReadInput,
	deadline time.Time,
) (browserop.ContentReadResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserop.ContentReadResult
		return zero, err
	}
	return browserop.DecodeContentReadResult(raw)
}
