package page

import (
	"context"
	"errors"
	"runtime"
	"strings"
	"unicode/utf8"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/input"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

type keyDefinition struct {
	key, code string
	virtual   int64
	text      string
}
type keyboardKey struct {
	keyDefinition
	modifier input.Modifier
}

// modifierBit interprets browser modifier names on the Host platform.
func modifierBit(name string) input.Modifier {
	switch name {
	case "Alt":
		return alt
	case "Control":
		return control
	case "Meta":
		return meta
	case "Shift":
		return shift
	case "ControlOrMeta":
		if runtime.GOOS == "darwin" {
			return meta
		}
		return control
	}
	return 0
}

// pointerModifiers combines the browser's schema-validated pointer modifiers.
func pointerModifiers(values *[]browserop.Modifier) input.Modifier {
	var mask input.Modifier
	if values != nil {
		for _, value := range *values {
			mask |= modifierBit(string(value))
		}
	}
	return mask
}

// namedKey looks up a key by its DOM key value, then by its physical code.
func namedKey(name string) (keyboardKey, error) {
	if name == "ControlOrMeta" {
		name = "Control"
		if runtime.GOOS == "darwin" {
			name = "Meta"
		}
	}
	if name == "Space" {
		name = " "
	}
	var found *keyDefinition
	for i := range keyDefinitions {
		if keyDefinitions[i].key == name {
			found = &keyDefinitions[i]
			break
		}
	}
	if found == nil {
		for i := range keyDefinitions {
			if keyDefinitions[i].code == name {
				found = &keyDefinitions[i]
				break
			}
		}
	}
	if found == nil {
		return keyboardKey{}, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "unknown keyboard key: " + name}
	}
	key := keyboardKey{*found, modifierBit(found.key)}
	if key.text == "" && utf8.RuneCountInString(key.key) == 1 {
		key.text = key.key
	}
	return key, nil
}

// combination validates all modifiers before any focus or native input changes.
func combination(value string) ([]keyboardKey, error) {
	keys := []keyboardKey{}
	for value != "+" {
		name, rest, ok := strings.Cut(value, "+")
		if !ok {
			break
		}
		modifier := modifierBit(name)
		duplicate := false
		for _, key := range keys {
			duplicate = duplicate || key.modifier == modifier
		}
		if modifier == 0 || duplicate {
			return nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "invalid keyboard modifier: " + name}
		}
		key, err := namedKey(name)
		if err != nil {
			return nil, err
		}
		keys = append(keys, key)
		value = rest
	}
	key, err := namedKey(value)
	if err != nil {
		return nil, err
	}
	return append(keys, key), nil
}

// event carries text only for printable keydown without a shortcut modifier.
func (key keyboardKey) event(kind input.KeyType, modifiers input.Modifier) *input.DispatchKeyEventParams {
	name, text := key.key, key.text
	shifted := false
	if modifiers&shift != 0 {
		for i := len(keyDefinitions) - 1; i >= 0; i-- {
			definition := keyDefinitions[i]
			if definition.code == key.code && utf8.RuneCountInString(definition.key) == 1 {
				name = definition.key
				shifted = true
				break
			}
		}
	}
	event := input.DispatchKeyEvent(kind).
		WithKey(name).
		WithCode(key.code).
		WithWindowsVirtualKeyCode(key.virtual).
		WithModifiers(modifiers)
	if kind == input.KeyDown {
		if modifiers&^shift == 0 {
			if shifted {
				text = name
			}
			event.Text = text
			event.UnmodifiedText = key.text
		}
		event.Commands = editingCommands(runtime.GOOS == "darwin", key.code, modifiers)
	}
	return event
}

// press releases every attempted key in reverse order after success or failure.
func press(ctx context.Context, tab *tabs.Tab, keys []keyboardKey, operation *cdp.Operation) error {
	pressed := []keyboardKey{}
	var modifiers input.Modifier
	var result error
	for _, key := range keys {
		modifiers |= key.modifier
		result = tab.Input(ctx, operation, func(work context.Context) error {
			pressed = append(pressed, key)
			operation.BeginInput()
			return key.event(input.KeyDown, modifiers).Do(protocol.WithExecutor(work, tab.Page()))
		})
		if result != nil {
			break
		}
	}
	var cleanupErr error
	for i := len(pressed) - 1; i >= 0; i-- {
		key := pressed[i]
		modifiers &^= key.modifier
		release := key.event(input.KeyUp, modifiers)
		cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
		if tab.Dialog().IsOpen() {
			err := tab.Dialog().Defer(cleanup, []tabs.InputRelease{&tabs.KeyRelease{Event: release}})
			cleanupErr = cdp.AfterCleanup(cleanupErr, err)
			if result == nil {
				result = &cdp.BrowserError{Kind: cdp.KindDialogBlocked}
			}
		} else {
			cleanupErr = cdp.AfterCleanup(cleanupErr, release.Do(protocol.WithExecutor(cleanup, tab.Page())))
		}
		cancel()
	}
	result = cdp.AfterCleanup(result, cleanupErr)
	if result == nil {
		operation.CompleteInput()
	}
	return result
}

// focus selects the target without synthesizing a mouse click.
func focus(ctx context.Context, element targetElement) error {
	focused, err := decodeElement[bool](ctx, element, scriptFocus, false, true)
	if err != nil {
		return err
	}
	if !focused {
		condition := "focused"
		return &cdp.BrowserError{Kind: cdp.KindNotActionable, Details: browserop.ErrorDetails{Condition: &condition}}
	}
	return nil
}

// typingFocus keeps targeted typing bound to its original element and document.
func typingFocus(ctx context.Context, element targetElement) error {
	focused, err := decodeElement[bool](ctx, element, scriptFocus, false, false)
	condition := "typing target changed or lost focus"
	if err != nil {
		var protocolErr *cdp.ProtocolError
		// Chrome has no typed stale-object/context code: match its exact messages.
		if !errors.As(err, &protocolErr) ||
			(protocolErr.Message != "Cannot find context with specified id" &&
				protocolErr.Message != "Could not find object with given id") {
			return err
		}
		condition = "typing document changed"
	} else if focused {
		return nil
	}
	return &cdp.BrowserError{Kind: cdp.KindNotActionable, Details: browserop.ErrorDetails{Condition: &condition}}
}

// typeFocused reports how many characters were delivered before a typing failure.
func typeFocused(
	ctx context.Context,
	tab *tabs.Tab,
	element *targetElement,
	text string,
	operation *cdp.Operation,
) error {
	delivered := uint(0)
	var result error
	for _, character := range text {
		if element != nil {
			result = typingFocus(ctx, *element)
			if result != nil {
				break
			}
		}
		name := string(character)
		if character == '\n' {
			name = "Enter"
		}
		key, err := namedKey(name)
		if err != nil {
			key = keyboardKey{keyDefinition: keyDefinition{key: string(character), text: string(character)}}
		}
		result = press(ctx, tab, []keyboardKey{key}, operation)
		if result != nil {
			break
		}
		delivered++
	}
	if result == nil && element != nil {
		result = typingFocus(ctx, *element)
	}
	if result != nil {
		details := cdp.ErrorDetails(result)
		details.Delivered = &delivered
		return &cdp.BrowserError{Kind: cdp.KindAction, Cause: result, Details: details}
	}
	return nil
}
