package page

import (
	"runtime"
	"strings"

	"github.com/chromedp/cdproto/input"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
)

const (
	alt     input.Modifier = 1
	control input.Modifier = 2
	meta    input.Modifier = 4
	shift   input.Modifier = 8
)

// ViewerKey returns the key event the Host receives for a viewer's key.
func ViewerKey(key browserproto.LiveViewerMessageKey, macViewer bool) *input.DispatchKeyEventParams {
	return viewerKey(key, macViewer, runtime.GOOS == "darwin")
}

// viewerKey translates a viewer key for the Host's editing conventions.
func viewerKey(key browserproto.LiveViewerMessageKey, macViewer, hostMac bool) *input.DispatchKeyEventParams {
	name, code := key.Key, key.Code
	keyCode := int64(key.KeyCode)
	modifiers := input.Modifier(key.Modifiers)
	var commands []string
	if macViewer && !hostMac {
		name, code, keyCode, modifiers, commands = viewerEditingKey(key, name, code, keyCode, modifiers)
	}
	kind := input.KeyRawDown
	if key.Action == "up" {
		kind = input.KeyUp
	}
	event := input.DispatchKeyEvent(kind).
		WithKey(name).
		WithCode(code).
		WithWindowsVirtualKeyCode(keyCode).
		WithModifiers(modifiers).
		WithLocation(int64(key.Location)).
		WithIsKeypad(key.Location == 3)
	if key.Action == "up" {
		return event
	}
	text := key.Text
	if key.Code == "Enter" || key.Code == "NumpadEnter" {
		carriage := "\r"
		text = &carriage
	} else if modifiers&(control|meta) != 0 && !key.AltGraph {
		text = nil
	}
	if text != nil {
		event.Type = input.KeyDown
		event.Text = *text
		event.UnmodifiedText = *text
	}
	event.AutoRepeat = key.Repeat
	if commands == nil {
		commands = editingCommands(hostMac, code, modifiers)
	}
	event.Commands = commands
	return event
}

// editingCommands supplies Blink commands for shortcuts otherwise handled by Cocoa menus.
func editingCommands(hostMac bool, code string, modifiers input.Modifier) []string {
	if !hostMac || modifiers&(meta|control|alt) != meta {
		return nil
	}
	if modifiers&shift != 0 {
		if code == "KeyZ" {
			return []string{"Redo"}
		}
		return nil
	}
	switch code {
	case "KeyA":
		return []string{"SelectAll"}
	case "KeyC":
		return []string{"Copy"}
	case "KeyX":
		return []string{"Cut"}
	case "KeyV":
		return []string{"Paste"}
	case "KeyZ":
		return []string{"Undo"}
	}
	return nil
}

// ClickModifiers maps Command-click from a Mac viewer to Control-click on Linux or Windows.
func ClickModifiers(modifiers input.Modifier, macViewer bool) input.Modifier {
	return clickModifiers(modifiers, macViewer, runtime.GOOS == "darwin")
}

// clickModifiers maps the viewer's new-tab gesture to the Host's modifier.
func clickModifiers(modifiers input.Modifier, macViewer, hostMac bool) input.Modifier {
	if macViewer && !hostMac && modifiers&meta != 0 {
		return modifiers&^meta | control
	}
	return modifiers
}

// PasteShortcut returns the Host's paste shortcut, down then up.
func PasteShortcut() [2]*input.DispatchKeyEventParams {
	return pasteShortcut(runtime.GOOS == "darwin")
}

// pasteShortcut creates native paste events for the Host platform.
func pasteShortcut(hostMac bool) [2]*input.DispatchKeyEventParams {
	modifiers := control
	if hostMac {
		modifiers = meta
	}
	down := input.DispatchKeyEvent(input.KeyRawDown).
		WithKey("v").
		WithCode("KeyV").
		WithWindowsVirtualKeyCode(86).
		WithModifiers(modifiers).
		WithCommands(editingCommands(hostMac, "KeyV", modifiers))
	up := input.DispatchKeyEvent(input.KeyUp).
		WithKey("v").
		WithCode("KeyV").
		WithWindowsVirtualKeyCode(86).
		WithModifiers(modifiers)
	return [2]*input.DispatchKeyEventParams{down, up}
}

func viewerEditingKey(
	key browserproto.LiveViewerMessageKey,
	name, code string,
	keyCode int64,
	modifiers input.Modifier,
) (string, string, int64, input.Modifier, []string) {
	var commands []string
	shifted := modifiers & shift
	if modifiers&meta != 0 {
		switch key.Code {
		case "ArrowLeft", "ArrowUp":
			name, code, keyCode = "Home", "Home", 36
			modifiers = shifted
			if key.Code == "ArrowUp" {
				modifiers |= control
			}
		case "ArrowRight", "ArrowDown":
			name, code, keyCode = "End", "End", 35
			modifiers = shifted
			if key.Code == "ArrowDown" {
				modifiers |= control
			}
		case "Backspace", "Delete":
			modifiers &^= meta | alt
			command := "DeleteToEndOfLine"
			if key.Code == "Backspace" {
				command = "DeleteToBeginningOfLine"
			}
			commands = []string{command}
		default:
			modifiers = modifiers&^meta | control
		}
	}
	if key.Code == "MetaLeft" || key.Code == "MetaRight" {
		name, code, keyCode = "Control", strings.ReplaceAll(key.Code, "Meta", "Control"), 17
	} else if modifiers&alt != 0 {
		switch key.Code {
		case "ArrowLeft", "ArrowRight", "Backspace", "Delete":
			modifiers = modifiers&^alt | control
		}
	}
	return name, code, keyCode, modifiers, commands
}
