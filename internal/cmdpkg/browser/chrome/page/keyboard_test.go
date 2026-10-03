package page

import (
	"reflect"
	"testing"

	"github.com/chromedp/cdproto/input"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// These table cases check the native key events that viewer keys produce; they
// run without timers, sockets, or a browser (under 1 s).
func TestViewerKeyboard(t *testing.T) {
	cases := []struct {
		name, code, key            string
		keyCode                    uint8
		modifiers                  input.Modifier
		text                       string
		mac, hostMac, altGraph, up bool
		location                   uint8
		wantKey, wantCode          string
		wantKeyCode                int64
		wantModifiers              input.Modifier
		wantText                   string
		commands                   []string
	}{
		{
			name:        "printable_keys_carry_their_text_so_the_page_gets_keypress",
			code:        "KeyA",
			key:         "a",
			keyCode:     65,
			text:        "a",
			wantKey:     "a",
			wantCode:    "KeyA",
			wantKeyCode: 65,
			wantText:    "a",
		},
		{
			name:        "enter_types_a_carriage_return",
			code:        "Enter",
			key:         "Enter",
			keyCode:     13,
			wantKey:     "Enter",
			wantCode:    "Enter",
			wantKeyCode: 13,
			wantText:    "\r",
		},
		{
			name:        "numpad_enter",
			code:        "NumpadEnter",
			key:         "Enter",
			keyCode:     13,
			location:    3,
			wantKey:     "Enter",
			wantCode:    "NumpadEnter",
			wantKeyCode: 13,
			wantText:    "\r",
		},
		{
			name:          "a_mac_viewer_uses_control_shortcuts_on_a_linux_host",
			code:          "KeyA",
			key:           "a",
			keyCode:       65,
			modifiers:     meta,
			mac:           true,
			wantKey:       "a",
			wantCode:      "KeyA",
			wantKeyCode:   65,
			wantModifiers: control,
		},
		{
			name:          "mac_redo",
			code:          "KeyZ",
			key:           "z",
			keyCode:       90,
			modifiers:     meta | shift,
			mac:           true,
			wantKey:       "z",
			wantCode:      "KeyZ",
			wantKeyCode:   90,
			wantModifiers: control | shift,
		},
		{
			name:          "mac_meta",
			code:          "MetaLeft",
			key:           "Meta",
			keyCode:       91,
			modifiers:     meta,
			mac:           true,
			wantKey:       "Control",
			wantCode:      "ControlLeft",
			wantKeyCode:   17,
			wantModifiers: control,
		},
		{
			name:        "mac_meta_release",
			code:        "MetaLeft",
			key:         "Meta",
			keyCode:     91,
			mac:         true,
			up:          true,
			wantKey:     "Control",
			wantCode:    "ControlLeft",
			wantKeyCode: 17,
		},
		{
			name:          "mac_text_navigation_maps_to_the_linux_keys",
			code:          "ArrowLeft",
			key:           "ArrowLeft",
			keyCode:       37,
			modifiers:     meta | shift,
			mac:           true,
			wantKey:       "Home",
			wantCode:      "Home",
			wantKeyCode:   36,
			wantModifiers: shift,
		},
		{
			name:          "document_end",
			code:          "ArrowDown",
			key:           "ArrowDown",
			keyCode:       40,
			modifiers:     meta,
			mac:           true,
			wantKey:       "End",
			wantCode:      "End",
			wantKeyCode:   35,
			wantModifiers: control,
		},
		{
			name:          "word",
			code:          "ArrowRight",
			key:           "ArrowRight",
			keyCode:       39,
			modifiers:     alt,
			mac:           true,
			wantKey:       "ArrowRight",
			wantCode:      "ArrowRight",
			wantKeyCode:   39,
			wantModifiers: control,
		},
		{
			name:          "delete_word",
			code:          "Backspace",
			key:           "Backspace",
			keyCode:       8,
			modifiers:     alt,
			mac:           true,
			wantKey:       "Backspace",
			wantCode:      "Backspace",
			wantKeyCode:   8,
			wantModifiers: control,
		},
		{
			name:        "delete_line",
			code:        "Backspace",
			key:         "Backspace",
			keyCode:     8,
			modifiers:   meta,
			mac:         true,
			wantKey:     "Backspace",
			wantCode:    "Backspace",
			wantKeyCode: 8,
			commands:    []string{"DeleteToBeginningOfLine"},
		},
		{
			name:          "option_and_altgr_characters_are_still_typed",
			code:          "Digit2",
			key:           "™",
			keyCode:       50,
			modifiers:     alt,
			mac:           true,
			text:          "™",
			wantKey:       "™",
			wantCode:      "Digit2",
			wantKeyCode:   50,
			wantModifiers: alt,
			wantText:      "™",
		},
		{
			name:          "altgr",
			code:          "KeyQ",
			key:           "@",
			keyCode:       81,
			modifiers:     alt | control,
			altGraph:      true,
			text:          "@",
			wantKey:       "@",
			wantCode:      "KeyQ",
			wantKeyCode:   81,
			wantModifiers: alt | control,
			wantText:      "@",
		},
		{
			name:          "a_macos_host_receives_editing_commands_for_menu_shortcuts",
			code:          "KeyA",
			key:           "a",
			keyCode:       65,
			modifiers:     meta,
			mac:           true,
			hostMac:       true,
			wantKey:       "a",
			wantCode:      "KeyA",
			wantKeyCode:   65,
			wantModifiers: meta,
			commands:      []string{"SelectAll"},
		},
		{
			name:          "host_redo",
			code:          "KeyZ",
			key:           "z",
			keyCode:       90,
			modifiers:     meta | shift,
			mac:           true,
			hostMac:       true,
			wantKey:       "z",
			wantCode:      "KeyZ",
			wantKeyCode:   90,
			wantModifiers: meta | shift,
			commands:      []string{"Redo"},
		},
		{
			name:          "host_shift_a",
			code:          "KeyA",
			key:           "a",
			keyCode:       65,
			modifiers:     meta | shift,
			mac:           true,
			hostMac:       true,
			wantKey:       "a",
			wantCode:      "KeyA",
			wantKeyCode:   65,
			wantModifiers: meta | shift,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			key := browserop.LiveViewerMessageKey{
				Action:    "down",
				Key:       tc.key,
				Code:      tc.code,
				KeyCode:   tc.keyCode,
				Modifiers: uint8(tc.modifiers),
				AltGraph:  tc.altGraph,
				Location:  tc.location,
			}
			if tc.text != "" {
				key.Text = &tc.text
			}
			if tc.up {
				key.Action = "up"
			}
			event := viewerKey(key, tc.mac, tc.hostMac)
			kind := input.KeyRawDown
			if tc.wantText != "" {
				kind = input.KeyDown
			}
			if tc.up {
				kind = input.KeyUp
			}
			if event.Type != kind || event.Key != tc.wantKey || event.Code != tc.wantCode ||
				event.WindowsVirtualKeyCode != tc.wantKeyCode ||
				event.Modifiers != tc.wantModifiers ||
				event.Text != tc.wantText ||
				event.UnmodifiedText != tc.wantText ||
				event.IsKeypad != (tc.location == 3) ||
				!reflect.DeepEqual(event.Commands, tc.commands) {
				t.Fatalf("event: %+v; case: %+v", event, tc)
			}
		})
	}
	t.Run("keys_without_a_character_stay_raw", func(t *testing.T) {
		for code, keyCode := range map[string]uint8{"Tab": 9, "Backspace": 8, "ArrowLeft": 37, "Escape": 27} {
			event := viewerKey(
				browserop.LiveViewerMessageKey{Action: "down", Code: code, Key: code, KeyCode: keyCode},
				false,
				false,
			)
			if event.Type != input.KeyRawDown || event.Text != "" {
				t.Fatalf("%s: %+v", code, event)
			}
		}
	})
	t.Run("other_viewers_keep_their_modifiers", func(t *testing.T) {
		for _, modifier := range []input.Modifier{control, meta} {
			event := viewerKey(
				browserop.LiveViewerMessageKey{Action: "down", Code: "KeyA", Key: "a", Modifiers: uint8(modifier)},
				false,
				false,
			)
			if event.Modifiers != modifier {
				t.Fatalf("modifiers: %v", event.Modifiers)
			}
		}
	})
	t.Run("command_click_becomes_control_click_on_a_linux_host", func(t *testing.T) {
		if clickModifiers(meta|shift, true, false) != control|shift || clickModifiers(meta, true, true) != meta ||
			clickModifiers(meta, false, false) != meta {
			t.Fatal("click modifier translation")
		}
	})
	t.Run("paste", func(t *testing.T) {
		mac, linux := pasteShortcut(true), pasteShortcut(false)
		if !reflect.DeepEqual(mac[0].Commands, []string{"Paste"}) || linux[0].Modifiers != control ||
			linux[0].Commands != nil ||
			mac[1].Type != input.KeyUp ||
			linux[1].Type != input.KeyUp {
			t.Fatal("paste events")
		}
	})
}
