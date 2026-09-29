package builtinproto

// The browser.* operations (docs/browser/browser.md): their limits, the values
// they share, and the tabs and elements they name. Each operation's input and
// result are in operations.go, and how one fails in failure.go.

import (
	"errors"
	"regexp"
)

// BrowserPrefix starts the name of every browser operation, such as
// browser.open.
const BrowserPrefix = "browser."

// The limits of the browser operations.
const (
	// DefaultTimeoutMS is the deadline of an operation whose input names none,
	// in milliseconds.
	DefaultTimeoutMS = 30_000
	// MaxTimeoutMS is the longest deadline an input may name, and open's
	// default: a cold start installs and launches Chrome first.
	MaxTimeoutMS = 300_000
	// MaxNodes is the most nodes, entries or matches one result lists.
	MaxNodes = 1_000
	// MaxNodeIndex is the largest zero-based match index.
	MaxNodeIndex = MaxNodes - 1
	// DefaultNodes is how many a result lists when the input names no limit.
	DefaultNodes = 100
	// InlineBytes is the largest result written to stdout; a larger one is
	// shortened or fails.
	InlineBytes = 64 * 1024
	// StdinChars is the longest text an input carries, such as typed text or
	// an expression, in Unicode scalar values.
	StdinChars = 1024 * 1024
	// ClipboardPNGBytes is the largest PNG a clipboard holds.
	ClipboardPNGBytes = 16 * 1024 * 1024
	// ClipboardPNGPixels is the most pixels of a clipboard's PNG.
	ClipboardPNGPixels = 16_000_000
	// FetchURLs is the most URLs one content.fetch reads.
	FetchURLs = 10
	// ConsoleEntries is the console entries a tab retains.
	ConsoleEntries = 1000
	// ConsoleBytes is the most bytes of the console entries a tab retains.
	ConsoleBytes = 1024 * 1024
	// CDPEvents is the CDP events a debugging connection retains.
	CDPEvents = 10000
	// CDPBytes is the most bytes of the CDP events a connection retains.
	CDPBytes = 8 * 1024 * 1024
	// LocatorLength is the longest locator, URL, path or name an input carries,
	// in Unicode scalar values.
	LocatorLength = 4096
)

// maxSafeInteger is the largest integer that JavaScript holds exactly, the
// largest number the model reads back.
const maxSafeInteger = 1<<53 - 1

// The pattern of a tab's identifier and of a node reference: a letter and a
// number from 1, which JavaScript holds exactly (docs/execution/runtime.md
// § Identifiers the model sees).
var (
	tabIDPattern   = regexp.MustCompile(`^t[1-9][0-9]{0,14}$`)
	nodeRefPattern = regexp.MustCompile(`^e[1-9][0-9]{0,14}$`)
)

// A TabID is a tab's public identity, which open and tabs return: t and the
// tab's number in the conversation (docs/browser/browser.md § One tab
// registry).
//
//demi:value
//demi:check pattern=tabIDPattern
//demi:describe A tab's public identity, which `open` and `tabs` return: `t` and the
//demi:describe tab's number in the conversation (`browser.md` § One tab registry).
type TabID string

// A NodeRef is a node reference, which inspect, find and probe return: e and a
// number unique within its tab. It stays valid while its document does.
//
//demi:value
//demi:check pattern=nodeRefPattern
//demi:describe A node reference, which `inspect`, `find` and `probe` return: `e` and
//demi:describe a number unique within its tab. It stays valid while its document
//demi:describe does.
type NodeRef string

// A Load says how far a navigation loads before it answers.
//
//demi:enum
//demi:describe How far a navigation loads before it answers.
type Load string

// The values of a [Load].
const (
	LoadCommit           Load = "commit"
	LoadDOMContentLoaded Load = "domcontentloaded"
	LoadLoad             Load = "load"
)

// An InspectView says which tree inspect returns.
//
//demi:enum
//demi:describe Which tree `inspect` returns.
type InspectView string

// The values of an [InspectView].
const (
	InspectAccessibility InspectView = "accessibility"
	InspectDOM           InspectView = "dom"
)

// A ReadProperty is what read reads from each element; --attribute reads an
// attribute instead.
//
//demi:enum
//demi:describe What `read` reads from each element; `--attribute` reads an attribute instead.
type ReadProperty string

// The values of a [ReadProperty].
const (
	ReadText        ReadProperty = "text"
	ReadTextContent ReadProperty = "text-content"
	ReadHTML        ReadProperty = "html"
	ReadValue       ReadProperty = "value"
	ReadVisible     ReadProperty = "visible"
	ReadEnabled     ReadProperty = "enabled"
	ReadChecked     ReadProperty = "checked"
)

// A Modifier is a modifier key held during pointer input. ControlOrMeta is Meta
// on macOS and Control elsewhere.
//
//demi:enum
//demi:describe A modifier key held during pointer input. `ControlOrMeta` is Meta on
//demi:describe macOS and Control elsewhere; the variant has no doc of its own, which
//demi:describe would make the set's JSON Schema a union.
type Modifier string

// The values of a [Modifier].
const (
	ModifierAlt           Modifier = "Alt"
	ModifierControl       Modifier = "Control"
	ModifierControlOrMeta Modifier = "ControlOrMeta"
	ModifierMeta          Modifier = "Meta"
	ModifierShift         Modifier = "Shift"
)

// A MouseButton is a pointer button; the left one by default.
//
//demi:enum
type MouseButton string

// The values of a [MouseButton].
const (
	ButtonLeft   MouseButton = "left"
	ButtonMiddle MouseButton = "middle"
	ButtonRight  MouseButton = "right"
)

// A TextCursor says where select-text leaves the cursor instead of selecting
// the text.
//
//demi:enum
//demi:describe Where `select-text` leaves the cursor instead of selecting the text.
type TextCursor string

// The values of a [TextCursor].
const (
	CursorBefore TextCursor = "before"
	CursorAfter  TextCursor = "after"
)

// An ElementState is the element condition wait waits for; visible by default.
//
//demi:enum
//demi:describe The element condition `wait` waits for.
type ElementState string

// The values of an [ElementState].
const (
	StateVisible  ElementState = "visible"
	StateHidden   ElementState = "hidden"
	StateAttached ElementState = "attached"
	StateDetached ElementState = "detached"
	StateEnabled  ElementState = "enabled"
)

// A ClipboardMime is a clipboard item's media type; text/plain by default.
//
//demi:enum
//demi:describe A clipboard item's media type.
type ClipboardMime string

// The values of a [ClipboardMime].
const (
	MimeTextPlain ClipboardMime = "text/plain"
	MimeTextHTML  ClipboardMime = "text/html"
	MimeImagePNG  ClipboardMime = "image/png"
)

// A ClipboardFormat is what clipboard.read returns inline: the text; without
// it, every item is written to files.
//
//demi:enum
//demi:describe What `clipboard.read` returns inline: the text; without it, every item
//demi:describe is written to files.
type ClipboardFormat string

// The values of a [ClipboardFormat].
const (
	FormatText ClipboardFormat = "text"
)

// A LogLevel is a console entry's level.
//
//demi:enum
//demi:describe A console entry's level.
type LogLevel string

// The values of a [LogLevel].
const (
	LevelDebug   LogLevel = "debug"
	LevelInfo    LogLevel = "info"
	LevelLog     LogLevel = "log"
	LevelWarning LogLevel = "warning"
	LevelError   LogLevel = "error"
)

// A ContentFormat is the form a page's content takes: rendered text, HTML, or
// the DOM tree serialized; text by default.
//
//demi:enum
//demi:describe The form a page's content takes: rendered text, HTML, or the DOM tree
//demi:describe serialized.
type ContentFormat string

// The values of a [ContentFormat].
const (
	ContentText ContentFormat = "text"
	ContentHTML ContentFormat = "html"
	ContentDOM  ContentFormat = "dom"
)

// An AssetKind is the kind of an asset a page uses.
//
//demi:enum
type AssetKind string

// The values of an [AssetKind].
const (
	AssetFont       AssetKind = "font"
	AssetImage      AssetKind = "image"
	AssetStylesheet AssetKind = "stylesheet"
	AssetVideo      AssetKind = "video"
)

// A DialogType is the kind of a JavaScript dialog.
//
//demi:enum
type DialogType string

// The values of a [DialogType].
const (
	DialogAlert        DialogType = "alert"
	DialogConfirm      DialogType = "confirm"
	DialogPrompt       DialogType = "prompt"
	DialogBeforeUnload DialogType = "beforeunload"
)

// A DialogOutcome is how a dialog was answered.
//
//demi:enum
type DialogOutcome string

// The values of a [DialogOutcome].
const (
	OutcomeAccepted  DialogOutcome = "accepted"
	OutcomeDismissed DialogOutcome = "dismissed"
)

// A ViewportMode says who decides a tab's viewport (docs/browser/live-view.md
// § Modes): the user's panel, a phone, or the agent; web by default.
//
//demi:enum
//demi:describe Who decides a tab's viewport (`live-view.md` § Modes): the user's
//demi:describe panel, a phone, or the agent.
type ViewportMode string

// The values of a [ViewportMode].
const (
	ViewportWeb    ViewportMode = "web"
	ViewportMobile ViewportMode = "mobile"
	ViewportCustom ViewportMode = "custom"
)

// A Locator is what an element target names an element by: a node reference,
// or a role, text, label, placeholder, test ID or CSS selector. It is the base
// of a query, whose own frame, container and index narrow it.
//
//demi:wire
type Locator struct {
	// A node reference returned by inspect or find
	Ref *NodeRef `json:"ref,omitzero"`
	// Accessible role, ASCII case-insensitive, such as button, textbox, or date
	Role *string `json:"role,omitzero" check:"chars=1..LocatorLength"`
	// Accessible name, with --role
	Name *string `json:"name,omitzero" check:"chars=1..LocatorLength"`
	// Accessible-name regular expression, with --role
	NamePattern *string `json:"name-pattern,omitzero" check:"chars=1..LocatorLength"`
	// Rendered-text regular expression
	TextPattern *string `json:"text-pattern,omitzero" check:"chars=1..LocatorLength"`
	// Associated label text (label for, wrapping label, or aria-labelledby)
	Label       *string `json:"label,omitzero" check:"chars=1..LocatorLength"`
	Placeholder *string `json:"placeholder,omitzero" check:"chars=1..LocatorLength"`
	// Visible text to match
	TextMatch *string `json:"text-match,omitzero" check:"chars=1..LocatorLength"`
	// data-testid attribute
	TestID *string `json:"test-id,omitzero" check:"chars=1..LocatorLength"`
	// CSS selector
	CSS *string `json:"css,omitzero" check:"chars=1..LocatorLength"`
	// Match the complete name or text
	Exact *bool `json:"exact,omitzero"`
}

// A BrowserTarget is an element target: a locator narrowed by frames, a
// container and an index. Admission checks which fields may combine
// (docs/browser/browser.md § Element targets).
//
//demi:wire
type BrowserTarget struct {
	Locator
	// Frame references, outermost to innermost
	Frame *[]NodeRef `json:"frame,omitzero"`
	// Explicit zero-based match index
	Nth *uint `json:"nth,omitzero" check:"range=..MaxNodeIndex"`
	// Container node reference
	Within *NodeRef `json:"within,omitzero"`
}

// ElementTarget returns the element target that an input with one carries, for
// the inputs that embed a [BrowserTarget].
func (t BrowserTarget) ElementTarget() BrowserTarget { return t }

// A BrowserQuery is a declarative query tree (docs/browser/browser.md
// § Queries): one base, match, and or or, narrowed by a container, a frame, what
// the element has or lacks, its text, its visibility and an index.
//
//demi:wire
type BrowserQuery struct {
	Match      *Locator        `json:"match,omitzero"`
	Within     *BrowserQuery   `json:"within,omitzero"`
	Frame      *BrowserQuery   `json:"frame,omitzero"`
	And        *[]BrowserQuery `json:"and,omitzero" check:"items=1.."`
	Or         *[]BrowserQuery `json:"or,omitzero" check:"items=1.."`
	Has        *BrowserQuery   `json:"has,omitzero"`
	HasNot     *BrowserQuery   `json:"hasNot,omitzero"`
	HasText    *string         `json:"hasText,omitzero" check:"chars=..StdinChars"`
	HasNotText *string         `json:"hasNotText,omitzero" check:"chars=..StdinChars"`
	Visible    *bool           `json:"visible,omitzero"`
	Nth        *uint           `json:"nth,omitzero"`
}

// check is the rule across a query's members: it has exactly one base. It runs
// for the query and for every query nested in it.
func (q BrowserQuery) check() error {
	bases := 0
	if q.Match != nil {
		bases++
	}
	if q.And != nil {
		bases++
	}
	if q.Or != nil {
		bases++
	}
	if bases != 1 {
		return errors.New("a query requires exactly one base: match, and, or")
	}
	return nil
}

// A NodeValue is a node's value: an input's text, or a range or progress
// number.
//
//demi:union untagged
//demi:describe A node's value: an input's text, or a range or progress number.
type NodeValue interface {
	nodeValue()
}

// A NodeText is a node's value that is text.
//
//demi:variant
type NodeText string

// A NodeNumber is a node's value that is a number.
//
//demi:variant
type NodeNumber float64

func (NodeText) nodeValue()   {}
func (NodeNumber) nodeValue() {}

// Bounds is a node's box in viewport CSS pixels.
//
//demi:wire
//demi:describe A node's box in viewport CSS pixels.
type Bounds struct {
	X      float64 `json:"x"`
	Y      float64 `json:"y"`
	Width  float64 `json:"width"`
	Height float64 `json:"height"`
}

// A BrowserNode is one accessibility node that find matched or probe found under
// a point.
//
//demi:wire
//demi:describe One accessibility node that `find` matched or `probe` found under a point.
type BrowserNode struct {
	Ref    *NodeRef   `json:"ref,omitzero"`
	Role   string     `json:"role"`
	Name   string     `json:"name"`
	Value  *NodeValue `json:"value,omitzero"`
	Depth  uint       `json:"depth"`
	States []string   `json:"states"`
	Bounds *Bounds    `json:"bounds,omitzero"`
}

// A BrowserTreeNode is one node of the tree inspect returns: an accessibility
// node, or a DOM element with its tag.
//
//demi:wire
//demi:describe One node of the tree `inspect` returns: an accessibility node, or a DOM
//demi:describe element with its tag.
type BrowserTreeNode struct {
	Ref      *NodeRef           `json:"ref,omitzero"`
	Role     *string            `json:"role,omitzero"`
	Name     *string            `json:"name,omitzero"`
	Value    *NodeValue         `json:"value,omitzero"`
	Tag      *string            `json:"tag,omitzero"`
	States   *[]string          `json:"states,omitzero"`
	Children *[]BrowserTreeNode `json:"children,omitzero"`
}

// A BrowserCreatedBy says who opened a tab: an agent, a page's window.open, a
// temporary command such as content.fetch, or the user.
//
//demi:union tag=kind
//demi:describe Who opened a tab: an agent, a page's `window.open`, a temporary command
//demi:describe such as `content.fetch`, or the user.
type BrowserCreatedBy interface {
	createdBy()
}

// An AgentCreator is an agent, by its number in the conversation.
//
//demi:variant agent
//demi:describe An agent, by its number in the conversation.
type AgentCreator struct {
	Number uint64 `json:"number" check:"range=..maxSafeInteger"`
}

// A PageCreator is a page that opened the tab.
//
//demi:variant page
type PageCreator struct {
	Opener TabID `json:"opener"`
}

// A TemporaryCreator is a temporary command of the agent with a number.
//
//demi:variant temporary
//demi:describe A temporary command of the agent with this number.
type TemporaryCreator struct {
	Number uint64 `json:"number" check:"range=..maxSafeInteger"`
}

// A UserCreator is the conversation's user.
//
//demi:variant user
type UserCreator struct{}

func (AgentCreator) createdBy()     {}
func (PageCreator) createdBy()      {}
func (TemporaryCreator) createdBy() {}
func (UserCreator) createdBy()      {}

// A tab as `tabs` lists it and the conversation browser tab routes return it.
//
//demi:wire
//demi:describe A tab as `tabs` lists it and the conversation browser tab routes return it.
//demi:export
type BrowserTab struct {
	ID        TabID            `json:"id"`
	Title     string           `json:"title"`
	URL       string           `json:"url"`
	CreatedBy BrowserCreatedBy `json:"createdBy"`
}

// A BrowserViewport is a tab's viewport (docs/browser/live-view.md § Modes): its
// CSS size, the pixel ratio it renders at, and who decides them.
//
//demi:wire
//demi:describe A tab's viewport (`live-view.md` § Modes): its CSS size, the pixel ratio
//demi:describe it renders at, and who decides them.
type BrowserViewport struct {
	Width            uint32       `json:"width" check:"range=1.."`
	Height           uint32       `json:"height" check:"range=1.."`
	DevicePixelRatio float64      `json:"devicePixelRatio" check:"range=2.2250738585072014e-308.."`
	Mode             ViewportMode `json:"mode"`
}

// A Dialog is a JavaScript dialog a tab shows.
//
//demi:wire
//demi:describe A JavaScript dialog a tab shows.
type Dialog struct {
	Type    DialogType `json:"type"`
	Message string     `json:"message"`
}
