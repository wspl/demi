package webapiproto

import (
	"github.com/wspl/demi/internal/commandproto"
)

// The most characters (Unicode scalar values) a shortcut's key sequence
// has.
const ShortcutMax = 64

// Who configures providers, fixed for the instance's lifetime
// (`product.md` § Instance mode).
// +demi:enum shared isolated
type InstanceMode string

// Values of the preceding enumeration.
const (
	InstanceModeShared   InstanceMode = "shared"
	InstanceModeIsolated InstanceMode = "isolated"
)

// `GET /settings`: the instance's fixed mode.
// +demi:root direction=receive output=web
// +demi:tolerant
type Settings struct {
	Mode InstanceMode `json:"mode"`
}

// Whether the page follows the system's light or dark scheme or fixes one.
// +demi:enum system light dark
type Theme string

// Values of the preceding enumeration.
const (
	ThemeSystem Theme = "system"
	ThemeLight  Theme = "light"
	ThemeDark   Theme = "dark"
)

// The page's neutral tone.
// +demi:enum ink warm
type Tone string

// Values of the preceding enumeration.
const (
	ToneInk  Tone = "ink"
	ToneWarm Tone = "warm"
)

// The page's accent color.
// +demi:enum blue purple pink red orange green teal
type Accent string

// Values of the preceding enumeration.
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
type Appearance struct {
	Theme  *Theme  `json:"theme,omitempty"`
	Tone   *Tone   `json:"tone,omitempty"`
	Accent *Accent `json:"accent,omitempty"`
	// The text size in pixels.
	// +demi:range min=12 max=18
	FontSize *uint8 `json:"fontSize,omitempty"`
}

// Keyboard shortcut overrides: each a key sequence the web app reads.
type Shortcuts struct {
	// A new conversation.
	// +demi:length chars max=64
	New *string `json:"new,omitempty"`
	// Showing or hiding the sidebar.
	// +demi:length chars max=64
	Sidebar *string `json:"sidebar,omitempty"`
	// Opening the settings.
	// +demi:length chars max=64
	Settings *string `json:"settings,omitempty"`
}

// A change to the shortcut overrides: a key sequence sets one, `null`
// removes it, and an absent one stays as saved.
type ShortcutsPatch struct {
	// +demi:length chars max=64
	// +demi:nullable
	New **string `json:"new,omitempty"`
	// +demi:length chars max=64
	// +demi:nullable
	Sidebar **string `json:"sidebar,omitempty"`
	// +demi:length chars max=64
	// +demi:nullable
	Settings **string `json:"settings,omitempty"`
}

// A user's saved preferences.
type Preferences struct {
	Appearance Appearance `json:"appearance"`
	Shortcuts  Shortcuts  `json:"shortcuts"`
	// The model settings a new conversation starts with, as the user last
	// chose them; existing conversations keep their own.
	LastModel *ModelSettings `json:"lastModel,omitempty"`
	// The time zone and languages the user's browser last reported, which
	// commands receive in their command context: a zone the backend knows,
	// in its IANA spelling, and each language once as its canonical tag.
	Locale *commandproto.CommandLocale `json:"locale,omitempty"`
}

// `PATCH /settings/preferences`: the overrides to change; everything absent
// stays as saved.
// +demi:root direction=send output=web
type PreferencesPatch struct {
	Appearance *Appearance     `json:"appearance,omitempty"`
	Shortcuts  *ShortcutsPatch `json:"shortcuts,omitempty"`
	LastModel  *ModelSettings  `json:"lastModel,omitempty"`
	// A time zone the backend does not know or a malformed language tag is
	// refused.
	Locale *commandproto.CommandLocale `json:"locale,omitempty"`
}

// `{ preferences }`: the answer of `GET` and `PATCH /settings/preferences`.
// +demi:root direction=receive output=web
// +demi:tolerant
type UserPreferences struct {
	Preferences Preferences `json:"preferences"`
}
