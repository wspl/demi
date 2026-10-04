package page

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// Info reads current tab information.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Info(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.InfoInput,
	deadline time.Time,
) (browserproto.InfoResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.InfoResult
		return zero, err
	}
	return browserproto.DecodeInfoResult(raw)
}

// Goto navigates to the requested URL.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Goto(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.GotoInput,
	deadline time.Time,
) (browserproto.NavigationResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.NavigationResult
		return zero, err
	}
	return browserproto.DecodeNavigationResult(raw)
}

// Back navigates to the previous history entry.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Back(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.BackInput,
	deadline time.Time,
) (browserproto.NavigationResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.NavigationResult
		return zero, err
	}
	return browserproto.DecodeNavigationResult(raw)
}

// Forward navigates to the next history entry.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Forward(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ForwardInput,
	deadline time.Time,
) (browserproto.NavigationResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.NavigationResult
		return zero, err
	}
	return browserproto.DecodeNavigationResult(raw)
}

// Reload reloads the current document.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Reload(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ReloadInput,
	deadline time.Time,
) (browserproto.NavigationResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.NavigationResult
		return zero, err
	}
	return browserproto.DecodeNavigationResult(raw)
}

// History reads the requested history page.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func History(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.HistoryInput,
	deadline time.Time,
) (browserproto.HistoryResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.HistoryResult
		return zero, err
	}
	return browserproto.DecodeHistoryResult(raw)
}

// Inspect observes the accessibility or DOM tree.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Inspect(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.InspectInput,
	deadline time.Time,
) (browserproto.InspectResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.InspectResult
		return zero, err
	}
	return browserproto.DecodeInspectResult(raw)
}

// Find resolves a locator or query and describes its matches.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Find(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.FindInput,
	deadline time.Time,
) (browserproto.FindResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.FindResult
		return zero, err
	}
	return browserproto.DecodeFindResult(raw)
}

// Read reads element properties while protecting password values.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Read(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ReadInput,
	deadline time.Time,
) (browserproto.ReadResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ReadResult
		return zero, err
	}
	return browserproto.DecodeReadResult(raw)
}

// Probe describes the elements at viewport coordinates.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Probe(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ProbeInput,
	deadline time.Time,
) (browserproto.ProbeResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ProbeResult
		return zero, err
	}
	return browserproto.DecodeProbeResult(raw)
}

// Click delivers a native click and observes its associated navigation.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Click(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ClickInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// Move moves the pointer to an element or viewport coordinates.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Move(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.MoveInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// Drag delivers a native pointer drag.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Drag(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.DragInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// Scroll delivers a native wheel event.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Scroll(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ScrollInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// Fill replaces the value of a fillable element.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Fill(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.FillInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// Type types text into a target or the focused element.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Type(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.TypeInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// Key delivers a validated key combination.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Key(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.KeyInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// Check sets and verifies the requested checked state.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Check(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.CheckInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// Select selects native options by value, label, or index.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Select(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.SelectInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// SelectText selects text in an element.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func SelectText(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.SelectTextInput,
	deadline time.Time,
) (browserproto.ActionResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ActionResult
		return zero, err
	}
	return browserproto.DecodeActionResult(raw)
}

// Wait waits for a URL, load, or element condition.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Wait(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.WaitInput,
	deadline time.Time,
) (browserproto.WaitResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.WaitResult
		return zero, err
	}
	return browserproto.DecodeWaitResult(raw)
}

// Eval evaluates a read-only expression with optional element targets.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Eval(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.EvalInput,
	deadline time.Time,
) (browserproto.EvalResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.EvalResult
		return zero, err
	}
	return browserproto.DecodeEvalResult(raw)
}

// Logs reads the tab console using its independent cursor.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func Logs(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.LogsInput,
	deadline time.Time,
) (browserproto.LogsResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.LogsResult
		return zero, err
	}
	return browserproto.DecodeLogsResult(raw)
}

// ViewportSet sets the custom viewport.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func ViewportSet(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ViewportSetInput,
	deadline time.Time,
) (browserproto.ViewportResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ViewportResult
		return zero, err
	}
	return browserproto.DecodeViewportResult(raw)
}

// ViewportReset restores the live-view viewport.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func ViewportReset(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ViewportResetInput,
	deadline time.Time,
) (browserproto.ViewportResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ViewportResult
		return zero, err
	}
	return browserproto.DecodeViewportResult(raw)
}

// DialogInspect reads the current dialog.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func DialogInspect(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.DialogInspectInput,
	deadline time.Time,
) (browserproto.DialogInspectResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.DialogInspectResult
		return zero, err
	}
	return browserproto.DecodeDialogInspectResult(raw)
}

// DialogAccept accepts the observed dialog.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func DialogAccept(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.DialogAcceptInput,
	deadline time.Time,
) (browserproto.DialogResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.DialogResult
		return zero, err
	}
	return browserproto.DecodeDialogResult(raw)
}

// DialogDismiss dismisses the observed dialog.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func DialogDismiss(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.DialogDismissInput,
	deadline time.Time,
) (browserproto.DialogResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.DialogResult
		return zero, err
	}
	return browserproto.DecodeDialogResult(raw)
}

// ContentRead reads document content in the requested format.
// It holds one tab admission through observation, input, cleanup, and associated waits.
func ContentRead(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ContentReadInput,
	deadline time.Time,
) (browserproto.ContentReadResult, error) {
	raw, err := Command(ctx, tab, &input, deadline)
	if err != nil {
		var zero browserproto.ContentReadResult
		return zero, err
	}
	return browserproto.DecodeContentReadResult(raw)
}
