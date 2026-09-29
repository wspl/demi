package webapi

import (
	"github.com/wspl/demi/go/commandservice"
)

// Who configures providers, fixed for the instance's lifetime
// (`product.md` § Instance mode).
//
//demi:enum
//demi:export
type InstanceMode string

const (
	InstanceModeShared   InstanceMode = "shared"
	InstanceModeIsolated InstanceMode = "isolated"
)

// `GET /settings`: the instance's fixed mode.
//
//demi:wire open
type Settings struct {
	Mode InstanceMode `json:"mode"`
}

// Whether the page follows the system's light or dark scheme or fixes one.
//
//demi:enum
//demi:export
type Theme string

const (
	ThemeSystem Theme = "system"
	ThemeLight  Theme = "light"
	ThemeDark   Theme = "dark"
)

// The page's neutral tone.
//
//demi:enum
//demi:export
type Tone string

const (
	ToneInk  Tone = "ink"
	ToneWarm Tone = "warm"
)

// The page's accent color.
//
//demi:enum
//demi:export
type Accent string

const (
	AccentBlue   Accent = "blue"
	AccentPurple Accent = "purple"
	AccentPink   Accent = "pink"
	AccentRed    Accent = "red"
	AccentOrange Accent = "orange"
	AccentGreen  Accent = "green"
	AccentTeal   Accent = "teal"
)

// Appearance overrides. In a patch, each field that is present replaces the
// saved one.
//
//demi:wire
type Appearance struct {
	Theme  *Theme  `json:"theme,omitzero"`
	Tone   *Tone   `json:"tone,omitzero"`
	Accent *Accent `json:"accent,omitzero"`
	// The text size in pixels.
	FontSize *uint8 `json:"fontSize,omitzero" check:"range=12..18"`
}

// Keyboard shortcut overrides: each a key sequence the browser reads.
//
//demi:wire
type Shortcuts struct {
	// A new conversation.
	New *string `json:"new,omitzero" check:"chars=..64"`
	// Showing or hiding the sidebar.
	Sidebar *string `json:"sidebar,omitzero" check:"chars=..64"`
	// Opening the settings.
	Settings *string `json:"settings,omitzero" check:"chars=..64"`
}

// A change to the shortcut overrides: a key sequence sets one, `null`
// removes it, and an absent one stays as saved.
//
//demi:wire
type ShortcutsPatch struct {
	New      **string `json:"new,omitzero" check:"nullable,chars=..64"`
	Sidebar  **string `json:"sidebar,omitzero" check:"nullable,chars=..64"`
	Settings **string `json:"settings,omitzero" check:"nullable,chars=..64"`
}

// A user's saved preferences.
//
//demi:wire
type Preferences struct {
	Appearance Appearance `json:"appearance"`
	Shortcuts  Shortcuts  `json:"shortcuts"`
	// The model settings a new conversation starts with, as the user last
	// chose them; existing conversations keep their own.
	LastModel *ModelSettings `json:"lastModel,omitzero"`
	// The time zone and languages the browser last reported, which
	// commands receive in their command context: a zone the backend knows,
	// in its IANA spelling, and each language once as its canonical tag.
	Locale *commandservice.CommandLocale `json:"locale,omitzero" check:"func=commandservice.Validate"`
}

// `PATCH /settings/preferences`: the overrides to change; everything absent
// stays as saved.
//
//demi:wire
type PreferencesPatch struct {
	Appearance *Appearance     `json:"appearance,omitzero"`
	Shortcuts  *ShortcutsPatch `json:"shortcuts,omitzero"`
	LastModel  *ModelSettings  `json:"lastModel,omitzero"`
	// A time zone the backend does not know or a malformed language tag is
	// refused.
	Locale *commandservice.CommandLocale `json:"locale,omitzero" check:"func=commandservice.Validate"`
}

// `{ preferences }`: the answer of `GET` and `PATCH /settings/preferences`.
//
//demi:wire open
type UserPreferences struct {
	Preferences Preferences `json:"preferences"`
}
