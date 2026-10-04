package live

import (
	"context"
	"log/slog"
	"slices"
	"sync/atomic"
	"unicode/utf16"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/input"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/page"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
)

type inputKind uint8

const (
	inputRelease inputKind = iota
	inputMessage
	inputWatch
	inputFinish
)

type inputItem struct {
	kind    inputKind
	message browserproto.LiveViewerMessage
	tab     *tabs.Tab
}
type viewerInput struct {
	items    chan inputItem
	overflow atomic.Bool
	done     chan struct{}
}

func startInput(ctx context.Context, environment *tabs.Environment, w *writer, mac bool) (*viewerInput, error) {
	in := &viewerInput{items: make(chan inputItem, 256), done: make(chan struct{})}
	err := environment.StartTask(func(owner context.Context) {
		defer close(in.done)
		runCtx, cancel := context.WithCancel(ctx)
		defer cancel()
		stop := context.AfterFunc(owner, cancel)
		defer stop()
		in.run(runCtx, w, mac)
	})
	if err != nil {
		return nil, err
	}
	return in, nil
}

func (in *viewerInput) send(message browserproto.LiveViewerMessage) {
	select {
	case in.items <- inputItem{kind: inputMessage, message: message}:
	default:
		in.overflow.Store(true)
	}
}

func (in *viewerInput) control(ctx context.Context, item inputItem) {
	select {
	case <-ctx.Done():
	case <-in.done:
	case in.items <- item:
	}
}

type heldInput struct {
	keys                map[string]*input.DispatchKeyEventParams
	buttons             []input.MouseButton
	x, y                float64
	touching, composing bool
}

func (in *viewerInput) run(ctx context.Context, w *writer, mac bool) {
	var tab *tabs.Tab
	var ahead inputItem
	var hasAhead bool
	held := heldInput{keys: make(map[string]*input.DispatchKeyEventParams)}
	defer func() {
		releaseInput(ctx, tab, &held)
	}()
	for {
		if ctx.Err() != nil {
			return
		}
		var item inputItem
		if hasAhead {
			item = ahead
			hasAhead = false
		} else {
			select {
			case <-ctx.Done():
				return
			case item = <-in.items:
			}
		}
		if item.kind == inputFinish {
			return
		}
		if in.overflow.Swap(false) {
			releaseInput(ctx, tab, &held)
			if item.kind == inputWatch {
				tab = item.tab
			}
			if in.drain(&tab) {
				return
			}
			continue
		}
		if item.kind == inputWatch {
			releaseInput(ctx, tab, &held)
			tab = item.tab
			continue
		}
		if item.kind == inputRelease {
			releaseInput(ctx, tab, &held)
			continue
		}
		if tab == nil {
			continue
		}
		item.message, ahead, hasAhead = newestInput(item.message, in.items)
		if err := deliverInput(ctx, tab, &held, item.message, mac, w); err != nil && tab.Context().Err() == nil {
			w.notice(ctx, "input_failed", err.Error())
		}
	}
}

// dispatchInput interrupts a CDP input wait when a dialog blocks the page.
func dispatchInput(ctx context.Context, tab *tabs.Tab, call func(context.Context) error) error {
	if tab.Dialog().IsOpen() {
		return nil
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	done := make(chan struct{})
	go func() {
		defer close(done)
		for {
			dialog, changed := tab.Dialog().Watch()
			if dialog != nil {
				cancel()
				return
			}
			select {
			case <-ctx.Done():
				return
			case <-changed:
			}
		}
	}()
	err := call(protocol.WithExecutor(ctx, tab.Page()))
	cancel()
	<-done
	if tab.Dialog().IsOpen() {
		return nil
	}
	return err
}

func deliverInput(
	ctx context.Context,
	tab *tabs.Tab,
	held *heldInput,
	message browserproto.LiveViewerMessage,
	mac bool,
	w *writer,
) error {
	target, accepts := inputTarget(message)
	if !accepts {
		return nil
	}
	if target != tab.ID() || tab.Dialog().IsOpen() {
		return nil
	}
	switch m := message.(type) {
	case *browserproto.LiveViewerMessagePointer:
		return deliverPointer(ctx, tab, held, m, mac)
	case *browserproto.LiveViewerMessageWheel:
		return dispatchInput(
			ctx,
			tab,
			input.DispatchMouseEvent(input.MouseWheel, m.X, m.Y).
				WithDeltaX(m.DeltaX).
				WithDeltaY(m.DeltaY).
				WithModifiers(input.Modifier(m.Modifiers)).
				Do,
		)
	case *browserproto.LiveViewerMessageKey:
		return deliverKey(ctx, tab, held, m, mac)
	case *browserproto.LiveViewerMessageText:
		held.composing = false
		return dispatchInput(ctx, tab, input.InsertText(m.Text).Do)
	case *browserproto.LiveViewerMessageComposition:
		if m.Text == "" && !held.composing {
			return nil
		}
		held.composing = m.Text != ""
		end := int64(len(utf16.Encode([]rune(m.Text))))
		return dispatchInput(ctx, tab, input.ImeSetComposition(m.Text, end, end).Do)
	case *browserproto.LiveViewerMessagePaste:
		return deliverPaste(ctx, tab, held, m)
	case *browserproto.LiveViewerMessageChoice:
		accepted, err := choose(ctx, tab, m.Token, m.Revision, m.Value, m.Indices)
		if err != nil {
			return err
		}
		w.control(ctx, &browserproto.LiveModuleMessageChoice{Token: m.Token, Accepted: accepted})
	case *browserproto.LiveViewerMessageHello,
		*browserproto.LiveViewerMessagePanel,
		*browserproto.LiveViewerMessageWatch,
		*browserproto.LiveViewerMessageMode,
		*browserproto.LiveViewerMessageUpload,
		*browserproto.LiveViewerMessageDialog,
		*browserproto.LiveViewerMessageAck,
		*browserproto.LiveViewerMessageKeyframe,
		*browserproto.LiveViewerMessageRelease:
	}
	return nil
}

func releaseInput(ctx context.Context, tab *tabs.Tab, held *heldInput) {
	old := *held
	*held = heldInput{keys: make(map[string]*input.DispatchKeyEventParams)}
	if tab == nil || tab.Context().Err() != nil {
		return
	}
	ctx, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
	defer cancel()
	var releases []tabs.InputRelease
	for _, pressed := range old.keys {
		event := input.DispatchKeyEvent(input.KeyUp)
		event.Key, event.Code = pressed.Key, pressed.Code
		event.WindowsVirtualKeyCode = pressed.WindowsVirtualKeyCode
		event.Location = pressed.Location
		event.IsKeypad = pressed.IsKeypad
		releases = append(releases, &tabs.KeyRelease{Event: event})
	}
	for _, button := range old.buttons {
		releases = append(
			releases,
			&tabs.MouseRelease{
				Event: input.DispatchMouseEvent(input.MouseReleased, old.x, old.y).
					WithButton(button).
					WithButtons(0).
					WithModifiers(0),
			},
		)
	}
	if tab.Dialog().IsOpen() {
		if err := tab.Dialog().Defer(ctx, releases); err != nil {
			slog.Warn("live view deferred input release", "error", err)
		}
		return
	}
	execution := protocol.WithExecutor(ctx, tab.Page())
	var err error
	for _, release := range releases {
		switch r := release.(type) {
		case *tabs.KeyRelease:
			err = r.Event.Do(execution)
		case *tabs.MouseRelease:
			err = r.Event.Do(execution)
		}
		if err != nil {
			break
		}
	}
	if err == nil && old.touching {
		err = input.DispatchTouchEvent(input.TouchCancel, []*input.TouchPoint{}).Do(execution)
	}
	if err == nil && old.composing {
		err = input.ImeSetComposition("", 0, 0).Do(execution)
	}
	if err != nil && tab.Context().Err() == nil {
		slog.Warn("live view input release", "tab", tab.ID(), "error", err)
	}
}

func deliverPointer(
	ctx context.Context,
	tab *tabs.Tab,
	held *heldInput,
	m *browserproto.LiveViewerMessagePointer,
	mac bool,
) error {
	held.x, held.y = m.X, m.Y
	if tab.Viewport().Mode == "mobile" {
		kind := input.TouchEnd
		switch {
		case m.Action == "down" && m.Button == "left":
			kind = input.TouchStart
		case m.Action == "move" && held.touching:
			kind = input.TouchMove
		case m.Action == "up" && held.touching:
		default:
			return nil
		}
		held.touching = kind != input.TouchEnd
		points := []*input.TouchPoint{}
		if held.touching {
			points = append(points, &input.TouchPoint{X: m.X, Y: m.Y})
		}
		return dispatchInput(ctx, tab, input.DispatchTouchEvent(kind, points).Do)
	}
	kind := input.MouseMoved
	button := input.MouseButton(m.Button)
	if m.Action == "down" {
		kind = input.MousePressed
		if !slices.Contains(held.buttons, button) {
			held.buttons = append(held.buttons, button)
		}
	}
	if m.Action == "up" {
		kind = input.MouseReleased
		held.buttons = slices.DeleteFunc(held.buttons, func(b input.MouseButton) bool {
			return b == button
		})
	}
	event := input.DispatchMouseEvent(kind, m.X, m.Y).
		WithButton(button).
		WithButtons(int64(m.Buttons)).
		WithClickCount(int64(m.ClickCount)).
		WithModifiers(page.ClickModifiers(input.Modifier(m.Modifiers), mac))
	return dispatchInput(ctx, tab, event.Do)
}

func (in *viewerInput) drain(tab **tabs.Tab) bool {
	var item inputItem
	draining := true
	for draining {
		select {
		case item = <-in.items:
			if item.kind == inputFinish {
				return true
			}
			if item.kind == inputWatch {
				*tab = item.tab
			}
		default:
			draining = false
		}
	}

	return false
}

func inputTarget(message browserproto.LiveViewerMessage) (browserproto.TabID, bool) {
	// Non-input messages are routed by the viewer, not this worker.
	var target browserproto.TabID
	switch m := message.(type) {
	case *browserproto.LiveViewerMessagePointer:
		target = m.Tab
	case *browserproto.LiveViewerMessageWheel:
		target = m.Tab
	case *browserproto.LiveViewerMessageKey:
		target = m.Tab
	case *browserproto.LiveViewerMessageText:
		target = m.Tab
	case *browserproto.LiveViewerMessageComposition:
		target = m.Tab
	case *browserproto.LiveViewerMessagePaste:
		target = m.Tab
	case *browserproto.LiveViewerMessageChoice:
		target = m.Tab
	case *browserproto.LiveViewerMessageHello,
		*browserproto.LiveViewerMessagePanel,
		*browserproto.LiveViewerMessageWatch,
		*browserproto.LiveViewerMessageMode,
		*browserproto.LiveViewerMessageUpload,
		*browserproto.LiveViewerMessageDialog,
		*browserproto.LiveViewerMessageAck,
		*browserproto.LiveViewerMessageKeyframe,
		*browserproto.LiveViewerMessageRelease:
		return "", false
	}

	return target, true
}

func deliverPaste(ctx context.Context, tab *tabs.Tab, held *heldInput, m *browserproto.LiveViewerMessagePaste) error {
	held.composing = false
	return dispatchInput(ctx, tab, func(execution context.Context) error {
		written, err := writeClipboard(execution, tab, m.Text, m.HTML)
		if err != nil {
			return err
		}
		if !written {
			return input.InsertText(m.Text).Do(execution)
		}
		for _, event := range page.PasteShortcut() {
			if err := event.Do(execution); err != nil {
				return err
			}
		}
		return nil
	})
}

func deliverKey(
	ctx context.Context,
	tab *tabs.Tab,
	held *heldInput,
	m *browserproto.LiveViewerMessageKey,
	mac bool,
) error {
	event := page.ViewerKey(*m, mac)
	if m.Action == "down" {
		held.keys[m.Code] = event
	} else if pressed := held.keys[m.Code]; pressed != nil {
		delete(held.keys, m.Code)
		event.Key, event.Code = pressed.Key, pressed.Code
		event.WindowsVirtualKeyCode = pressed.WindowsVirtualKeyCode
		event.Location = pressed.Location
		event.IsKeypad = pressed.IsKeypad
	}
	return dispatchInput(ctx, tab, event.Do)
}
