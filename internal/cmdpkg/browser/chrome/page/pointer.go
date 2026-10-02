package page

import (
	"context"
	"math"
	"strconv"
	"strings"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/input"
	chrome "github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

type point struct{ x, y float64 }

// coordinates validates finite CSS coordinates against the current viewport.
func coordinates(ctx context.Context, tab *tabs.Tab, value string) (point, error) {
	x, y, ok := strings.Cut(value, ",")
	if !ok {
		return point{}, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "coordinates must be x,y"}
	}
	a, err := strconv.ParseFloat(strings.TrimSpace(x), 64)
	if err != nil {
		return point{}, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "invalid x coordinate", Cause: err}
	}
	b, err := strconv.ParseFloat(strings.TrimSpace(y), 64)
	if err != nil {
		return point{}, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "invalid y coordinate", Cause: err}
	}
	if math.IsNaN(a) || math.IsNaN(b) || math.IsInf(a, 0) || math.IsInf(b, 0) || a < 0 || b < 0 {
		return point{}, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "coordinates must be finite, nonnegative viewport pixels"}
	}
	_, _, _, viewport, _, _, err := chrome.GetLayoutMetrics().Do(protocol.WithExecutor(ctx, tab.Page()))
	if err != nil {
		return point{}, err
	}
	if a >= float64(viewport.ClientWidth) || b >= float64(viewport.ClientHeight) {
		return point{}, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "coordinates must lie inside the current viewport"}
	}
	return point{a, b}, nil
}

// releaseMouse transfers releases blocked by a dialog to the tab owner.
func releaseMouse(ctx context.Context, tab *tabs.Tab, result error, release *input.DispatchMouseEventParams) error {
	if tab.Context().Err() != nil {
		return result
	}
	cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
	defer cancel()
	if tab.Dialog().IsOpen() {
		err := tab.Dialog().Defer(cleanup, []tabs.InputRelease{&tabs.MouseRelease{Event: release}})
		return cdp.AfterCleanup(cdp.AfterCleanup(result, &cdp.BrowserError{Kind: cdp.KindDialogBlocked}), err)
	}
	return cdp.AfterCleanup(result, release.Do(protocol.WithExecutor(cleanup, tab.Page())))
}

// clickAt delivers native pointer input and releases the button on every exit.
func clickAt(ctx context.Context, tab *tabs.Tab, p point, button input.MouseButton, count int64, modifiers input.Modifier, operation *cdp.Operation) error {
	err := tab.Input(ctx, operation, func(work context.Context) error {
		operation.BeginInput()
		executor := protocol.WithExecutor(work, tab.Page())
		if err := input.DispatchMouseEvent(input.MouseMoved, p.x, p.y).Do(executor); err != nil {
			return err
		}
		if err := input.DispatchMouseEvent(input.MousePressed, p.x, p.y).WithButton(button).WithClickCount(count).WithModifiers(modifiers).Do(executor); err != nil {
			return err
		}
		return input.DispatchMouseEvent(input.MouseReleased, p.x, p.y).WithButton(button).WithClickCount(count).WithModifiers(modifiers).Do(executor)
	})
	if err == nil {
		operation.CompleteInput()
		return nil
	}
	return releaseMouse(ctx, tab, err, input.DispatchMouseEvent(input.MouseReleased, p.x, p.y).WithButton(button).WithClickCount(count))
}
