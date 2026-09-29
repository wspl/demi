package builtinproto

// Each browser.* operation's input and result (docs/browser/browser.md
// § Results). Inputs are the flat argument objects the command's flags and
// positionals make; results are what --json prints.

import (
	"encoding/json/jsontext"
	"time"
)

// A BrowserInput is the input of a browser operation. Every input has a
// deadline; the inputs that name a tab have a [Tabbed], those that name an
// element a [BrowserTarget].
type BrowserInput interface {
	// Deadline returns the whole operation's deadline: the one the input names,
	// or the operation's default.
	Deadline() time.Duration
}

// Timed is the deadline every input takes.
//
//demi:wire
type Timed struct {
	// Whole operation deadline in milliseconds
	Timeout *uint64 `json:"timeout,omitzero" check:"range=1..MaxTimeoutMS"`
}

// Deadline returns the deadline the input names, or [DefaultTimeoutMS].
func (t Timed) Deadline() time.Duration {
	return t.deadline(DefaultTimeoutMS)
}

func (t Timed) deadline(defaultMS uint64) time.Duration {
	if t.Timeout == nil {
		return time.Duration(defaultMS) * time.Millisecond
	}
	return time.Duration(*t.Timeout) * time.Millisecond
}

// Tabbed is the tab an operation acts on; open, tabs and content.fetch act on
// none.
//
//demi:wire
type Tabbed struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
}

// TabID returns the tab the input names.
func (t Tabbed) TabID() TabID { return t.Tab }

// OpenInput is the input of browser.open, which opens a tab at a URL, starting
// the browser when it does not run.
//
//demi:wire
//demi:schema
//demi:describe `open`: opens a tab at a URL, starting the browser when it does not run.
type OpenInput struct {
	URL  string `json:"url" check:"chars=1..LocatorLength"`
	Load *Load  `json:"load,omitzero"`
	Timed
}

// Deadline returns the deadline the input names, or [MaxTimeoutMS]: a cold
// start installs and launches Chrome before the page loads.
func (i OpenInput) Deadline() time.Duration {
	return i.deadline(MaxTimeoutMS)
}

// OpenResult is what open answers: the new tab, with its title and viewport
// when the page reported them in time.
//
//demi:wire
//demi:schema
//demi:describe What `open` answers: the new tab, with its title and viewport when the
//demi:describe page reported them in time.
type OpenResult struct {
	Tab      TabID            `json:"tab"`
	URL      string           `json:"url"`
	Title    *string          `json:"title,omitzero"`
	Viewport *BrowserViewport `json:"viewport,omitzero"`
}

// TabsInput is the input of browser.tabs, which lists the browser's tabs.
//
//demi:wire
//demi:schema
//demi:describe `tabs`: lists the browser's tabs.
type TabsInput struct {
	Offset *uint `json:"offset,omitzero"`
	Limit  *uint `json:"limit,omitzero" check:"range=1..MaxNodes"`
	Timed
}

// TabsResult is what tabs answers.
//
//demi:wire
//demi:schema
type TabsResult struct {
	Tabs      []BrowserTab `json:"tabs"`
	Truncated bool         `json:"truncated"`
}

// InfoInput is the input of browser.info, which reads a tab's URL, title,
// viewport and dialog.
//
//demi:wire
//demi:schema
//demi:describe `info`: reads a tab's URL, title, viewport and dialog.
type InfoInput struct {
	Tabbed
	Timed
}

// InfoResult is what info answers.
//
//demi:wire
//demi:schema
type InfoResult struct {
	Tab      TabID           `json:"tab"`
	URL      string          `json:"url"`
	Title    string          `json:"title"`
	Viewport BrowserViewport `json:"viewport"`
	Dialog   *Dialog         `json:"dialog,omitzero"`
}

// GotoInput is the input of browser.goto, which navigates a tab to a URL.
//
//demi:wire
//demi:schema
//demi:describe `goto`: navigates a tab to a URL.
type GotoInput struct {
	Tabbed
	URL  string `json:"url" check:"chars=1..LocatorLength"`
	Load *Load  `json:"load,omitzero"`
	Timed
}

// BackInput is the input of browser.back, which goes one entry back in a tab's
// history.
//
//demi:wire
//demi:schema
//demi:describe `back`: goes one entry back in a tab's history.
type BackInput struct {
	Tabbed
	Load *Load `json:"load,omitzero"`
	Timed
}

// ForwardInput is the input of browser.forward, which goes one entry forward in
// a tab's history.
//
//demi:wire
//demi:schema
//demi:describe `forward`: goes one entry forward in a tab's history.
type ForwardInput struct {
	Tabbed
	Load *Load `json:"load,omitzero"`
	Timed
}

// ReloadInput is the input of browser.reload, which reloads a tab's document.
//
//demi:wire
//demi:schema
//demi:describe `reload`: reloads a tab's document.
type ReloadInput struct {
	Tabbed
	Load *Load `json:"load,omitzero"`
	Timed
}

// NavigationResult is what a navigation answers: the URL it observed, with the
// title when the same document reported it in time.
//
//demi:wire
//demi:schema
//demi:describe What a navigation answers: the URL it observed, with the title when the
//demi:describe same document reported it in time.
type NavigationResult struct {
	Tab   TabID   `json:"tab"`
	URL   string  `json:"url"`
	Title *string `json:"title,omitzero"`
}

// HistoryInput is the input of browser.history, which lists a tab's navigation
// entries.
//
//demi:wire
//demi:schema
//demi:describe `history`: lists a tab's navigation entries.
type HistoryInput struct {
	Tabbed
	Offset *uint `json:"offset,omitzero"`
	Limit  *uint `json:"limit,omitzero" check:"range=1..MaxNodes"`
	Timed
}

// A HistoryEntry is one navigation entry of a tab.
//
//demi:wire
type HistoryEntry struct {
	Index   uint   `json:"index"`
	URL     string `json:"url"`
	Title   string `json:"title"`
	Current bool   `json:"current"`
}

// HistoryResult is what history answers.
//
//demi:wire
//demi:schema
type HistoryResult struct {
	Entries   []HistoryEntry `json:"entries"`
	Truncated bool           `json:"truncated"`
}

// CloseInput is the input of browser.close, which closes a tab.
//
//demi:wire
//demi:schema
//demi:describe `close`: closes a tab.
type CloseInput struct {
	Tabbed
	Timed
}

// CloseResult is what close answers.
//
//demi:wire
//demi:schema
type CloseResult struct {
	Closed TabID `json:"closed"`
}

// InspectInput is the input of browser.inspect, which reads a tab's
// accessibility or DOM tree.
//
//demi:wire
//demi:schema
//demi:describe `inspect`: reads a tab's accessibility or DOM tree.
type InspectInput struct {
	Tabbed
	View *InspectView `json:"view,omitzero"`
	// Container node reference
	Within *NodeRef `json:"within,omitzero"`
	// Frame references, outermost to innermost
	Frame *[]NodeRef `json:"frame,omitzero"`
	Limit *uint      `json:"limit,omitzero" check:"range=1..MaxNodes"`
	Timed
}

// InspectResult is what inspect answers.
//
//demi:wire
//demi:schema
type InspectResult struct {
	Tab       TabID             `json:"tab"`
	URL       string            `json:"url"`
	Title     string            `json:"title"`
	View      InspectView       `json:"view"`
	Tree      []BrowserTreeNode `json:"tree"`
	Truncated bool              `json:"truncated"`
}

// FindInput is the input of browser.find, which lists the elements a target or
// a query tree matches.
//
//demi:wire
//demi:schema
//demi:describe `find`: lists the elements a target or a query tree matches.
type FindInput struct {
	Tabbed
	BrowserTarget
	Offset *uint `json:"offset,omitzero"`
	Limit  *uint `json:"limit,omitzero" check:"range=1..MaxNodes"`
	// Read a declarative query tree from stdin
	Query *bool `json:"query,omitzero"`
	// JSON query tree when --query is supplied
	Body *string `json:"body,omitzero" check:"chars=..StdinChars"`
	Timed
}

// FindResult is what find answers. Count is every current match, even when
// offset and limit return fewer.
//
//demi:wire
//demi:schema
//demi:describe What `find` answers. `count` is every current match, even when `offset`
//demi:describe and `limit` return fewer.
type FindResult struct {
	Matches   []BrowserNode `json:"matches"`
	Count     uint          `json:"count"`
	Truncated bool          `json:"truncated"`
}

// ReadInput is the input of browser.read, which reads a property or an
// attribute of the target's element, or of every match with --all.
//
//demi:wire
//demi:schema
//demi:describe `read`: reads a property or an attribute of the target's element, or
//demi:describe of every match with `--all`.
type ReadInput struct {
	Tabbed
	BrowserTarget
	Property  *ReadProperty `json:"property,omitzero"`
	Attribute *string       `json:"attribute,omitzero" check:"chars=1..LocatorLength"`
	All       *bool         `json:"all,omitzero"`
	Timed
}

// A ReadResult is what read answers: one value, or every match's with --all.
//
//demi:union untagged
//demi:schema
//demi:describe What `read` answers: one value, or every match's with `--all`.
type ReadResult interface {
	readResult()
}

// ReadOne is what read answers for one element.
//
//demi:variant
type ReadOne struct {
	Value jsontext.Value `json:"value"`
}

// ReadAll is what read answers with --all.
//
//demi:variant
type ReadAll struct {
	Values    []jsontext.Value `json:"values"`
	Truncated bool             `json:"truncated"`
}

func (ReadOne) readResult() {}
func (ReadAll) readResult() {}

// ScreenshotInput is the input of browser.screenshot, which captures a tab's
// viewport, whole page or a rectangle as PNG.
//
//demi:wire
//demi:schema
//demi:describe `screenshot`: captures a tab's viewport, whole page or a rectangle as PNG.
type ScreenshotInput struct {
	Tabbed
	// New output file on this Host
	Output    *string `json:"output,omitzero" check:"chars=1..LocatorLength"`
	Overwrite *bool   `json:"overwrite,omitzero"`
	FullPage  *bool   `json:"full-page,omitzero"`
	// CSS rectangle: x,y,width,height
	Clip *string `json:"clip,omitzero" check:"chars=1..LocatorLength"`
	Timed
}

// An ImageMime is the media type of a screenshot.
//
//demi:enum
//demi:describe The media type of a screenshot.
type ImageMime string

// The values of an [ImageMime].
const (
	ImagePNG ImageMime = "image/png"
)

// ScreenshotResult is what screenshot answers when it writes a file; width and
// height are in CSS pixels.
//
//demi:wire
//demi:schema
//demi:describe What `screenshot` answers when it writes a file; `width` and `height` are
//demi:describe in CSS pixels.
type ScreenshotResult struct {
	Path     string          `json:"path"`
	MimeType ImageMime       `json:"mimeType"`
	Width    uint32          `json:"width"`
	Height   uint32          `json:"height"`
	Viewport BrowserViewport `json:"viewport"`
}

// ProbeInput is the input of browser.probe, which lists the nodes under a
// viewport point.
//
//demi:wire
//demi:schema
//demi:describe `probe`: lists the nodes under a viewport point.
type ProbeInput struct {
	Tabbed
	// Viewport CSS coordinates: x,y
	XY                     string `json:"xy" check:"chars=1..LocatorLength"`
	IncludeNonInteractable *bool  `json:"include-non-interactable,omitzero"`
	// New output file on this Host
	Output    *string `json:"output,omitzero" check:"chars=1..LocatorLength"`
	Overwrite *bool   `json:"overwrite,omitzero"`
	Timed
}

// ProbeResult is what probe answers.
//
//demi:wire
//demi:schema
type ProbeResult struct {
	Matches   []BrowserNode   `json:"matches"`
	Viewport  BrowserViewport `json:"viewport"`
	Path      *string         `json:"path,omitzero"`
	Truncated bool            `json:"truncated"`
}

// ClickInput is the input of browser.click, which clicks the target's element
// or a viewport point.
//
//demi:wire
//demi:schema
//demi:describe `click`: clicks the target's element or a viewport point.
type ClickInput struct {
	Tabbed
	BrowserTarget
	// Viewport CSS coordinates: x,y
	XY       *string      `json:"xy,omitzero" check:"chars=1..LocatorLength"`
	Modifier *[]Modifier  `json:"modifier,omitzero"`
	Count    *uint8       `json:"count,omitzero" check:"range=1..2"`
	Button   *MouseButton `json:"button,omitzero"`
	// Expected URL glob after the action
	WaitURL *string `json:"wait-url,omitzero" check:"chars=1..LocatorLength"`
	Timed
}

// MoveInput is the input of browser.move, which moves the pointer over the
// target's element or to a point.
//
//demi:wire
//demi:schema
//demi:describe `move`: moves the pointer over the target's element or to a point.
type MoveInput struct {
	Tabbed
	BrowserTarget
	// Viewport CSS coordinates: x,y
	XY       *string     `json:"xy,omitzero" check:"chars=1..LocatorLength"`
	Modifier *[]Modifier `json:"modifier,omitzero"`
	Timed
}

// DragInput is the input of browser.drag, which drags the pointer through
// viewport points.
//
//demi:wire
//demi:schema
//demi:describe `drag`: drags the pointer through viewport points.
type DragInput struct {
	Tabbed
	// Viewport CSS coordinates x,y, at least two
	Point    []string    `json:"point" check:"items=2..,each(chars=1..LocatorLength)"`
	Modifier *[]Modifier `json:"modifier,omitzero"`
	Timed
}

// ScrollInput is the input of browser.scroll, which scrolls at the target's
// element or a point.
//
//demi:wire
//demi:schema
//demi:describe `scroll`: scrolls at the target's element or a point.
type ScrollInput struct {
	Tabbed
	BrowserTarget
	// Viewport CSS coordinates: x,y
	XY       *string     `json:"xy,omitzero" check:"chars=1..LocatorLength"`
	Modifier *[]Modifier `json:"modifier,omitzero"`
	DX       *float64    `json:"dx,omitzero"`
	DY       *float64    `json:"dy,omitzero"`
	Timed
}

// FillInput is the input of browser.fill, which replaces the value of the
// target's field.
//
//demi:wire
//demi:schema
//demi:describe `fill`: replaces the value of the target's field.
type FillInput struct {
	Tabbed
	BrowserTarget
	Text string `json:"text" check:"chars=..StdinChars"`
	// Expected URL glob after the action
	WaitURL *string `json:"wait-url,omitzero" check:"chars=1..LocatorLength"`
	Timed
}

// TypeInput is the input of browser.type, which types text key by key, into the
// target or the focused element.
//
//demi:wire
//demi:schema
//demi:describe `type`: types text key by key, into the target or the focused element.
type TypeInput struct {
	Tabbed
	BrowserTarget
	Text string `json:"text" check:"chars=..StdinChars"`
	Timed
}

// KeyInput is the input of browser.key, which presses a key or a chord.
//
//demi:wire
//demi:schema
//demi:describe `key`: presses a key or a chord.
type KeyInput struct {
	Tabbed
	BrowserTarget
	Key string `json:"key" check:"chars=1..LocatorLength"`
	// Expected URL glob after the action
	WaitURL *string `json:"wait-url,omitzero" check:"chars=1..LocatorLength"`
	Timed
}

// CheckInput is the input of browser.check, which checks or unchecks the
// target's checkbox or radio button.
//
//demi:wire
//demi:schema
//demi:describe `check`: checks or unchecks the target's checkbox or radio button.
type CheckInput struct {
	Tabbed
	BrowserTarget
	Value bool `json:"value"`
	Timed
}

// SelectInput is the input of browser.select, which selects options of the
// target's select element by value, label or index.
//
//demi:wire
//demi:schema
//demi:describe `select`: selects options of the target's select element by value,
//demi:describe label or index.
type SelectInput struct {
	Tabbed
	BrowserTarget
	Value       *[]string `json:"value,omitzero" check:"each(chars=..StdinChars)"`
	OptionLabel *[]string `json:"option-label,omitzero" check:"each(chars=..StdinChars)"`
	OptionIndex *[]uint   `json:"option-index,omitzero"`
	Timed
}

// SelectTextInput is the input of browser.select-text, which selects text
// inside the target, or places the cursor before or after it.
//
//demi:wire
//demi:schema
//demi:describe `select-text`: selects text inside the target, or places the cursor
//demi:describe before or after it.
type SelectTextInput struct {
	Tabbed
	BrowserTarget
	Text   string      `json:"text" check:"chars=..StdinChars"`
	Cursor *TextCursor `json:"cursor,omitzero"`
	Prefix *string     `json:"prefix,omitzero" check:"chars=..StdinChars"`
	Suffix *string     `json:"suffix,omitzero" check:"chars=..StdinChars"`
	Timed
}

// ActionResult is what a pointer or form action answers: the operation and the
// target it acted on, its own result, and what it observed after: the URL, tabs
// the page opened, and a dialog.
//
//demi:wire
//demi:schema
//demi:describe What a pointer or form action answers: the operation and the target it
//demi:describe acted on, its own result, and what it observed after: the URL, tabs the
//demi:describe page opened, and a dialog.
type ActionResult struct {
	Operation  string         `json:"operation"`
	Target     *string        `json:"target,omitzero"`
	Result     jsontext.Value `json:"result"`
	URL        *string        `json:"url,omitzero"`
	OpenedTabs *[]TabID       `json:"openedTabs,omitzero"`
	Dialog     *Dialog        `json:"dialog,omitzero"`
}

// WaitInput is the input of browser.wait, which waits for a URL, the current
// document's load, or an element condition.
//
//demi:wire
//demi:schema
//demi:describe `wait`: waits for a URL, the current document's load, or an element
//demi:describe condition.
type WaitInput struct {
	Tabbed
	BrowserTarget
	URL   *string       `json:"url,omitzero" check:"chars=1..LocatorLength"`
	Load  *Load         `json:"load,omitzero"`
	State *ElementState `json:"state,omitzero"`
	Timed
}

// WaitResult is what wait answers; a load wait carries neither url nor ref.
//
//demi:wire
//demi:schema
//demi:describe What `wait` answers; a load wait carries neither `url` nor `ref`.
type WaitResult struct {
	Condition string   `json:"condition"`
	Matched   bool     `json:"matched"`
	URL       *string  `json:"url,omitzero"`
	Ref       *NodeRef `json:"ref,omitzero"`
}

// UploadInput is the input of browser.upload, which sets the files of the
// target's file input.
//
//demi:wire
//demi:schema
//demi:describe `upload`: sets the files of the target's file input.
type UploadInput struct {
	Tabbed
	BrowserTarget
	File []string `json:"file" check:"items=1..,each(chars=1..LocatorLength)"`
	Timed
}

// UploadResult is what upload answers.
//
//demi:wire
//demi:schema
type UploadResult struct {
	Files    []string `json:"files"`
	Attached uint     `json:"attached"`
}

// DownloadInput is the input of browser.download, which clicks the target or a
// point and saves the download it starts.
//
//demi:wire
//demi:schema
//demi:describe `download`: clicks the target or a point and saves the download it starts.
type DownloadInput struct {
	Tabbed
	BrowserTarget
	// Viewport CSS coordinates: x,y
	XY       *string     `json:"xy,omitzero" check:"chars=1..LocatorLength"`
	Modifier *[]Modifier `json:"modifier,omitzero"`
	// New output file on this Host
	Output    *string `json:"output,omitzero" check:"chars=1..LocatorLength"`
	Overwrite *bool   `json:"overwrite,omitzero"`
	Timed
}

// DownloadResult is what download answers.
//
//demi:wire
//demi:schema
type DownloadResult struct {
	Path              string `json:"path"`
	SuggestedFilename string `json:"suggestedFilename"`
	Bytes             uint64 `json:"bytes"`
	MimeType          string `json:"mimeType"`
}

// ClipboardWriteInput is the input of browser.clipboard.write, which writes
// stdin to the tab's clipboard.
//
//demi:wire
//demi:schema
//demi:describe `clipboard.write`: writes stdin to the tab's clipboard.
type ClipboardWriteInput struct {
	Tabbed
	Mime *ClipboardMime `json:"mime,omitzero"`
	Timed
}

// ClipboardWriteResult is what clipboard.write answers.
//
//demi:wire
//demi:schema
type ClipboardWriteResult struct {
	MimeType ClipboardMime `json:"mimeType"`
	Bytes    uint          `json:"bytes"`
}

// ClipboardReadInput is the input of browser.clipboard.read, which reads the
// tab's clipboard as text, or writes every item to a directory.
//
//demi:wire
//demi:schema
//demi:describe `clipboard.read`: reads the tab's clipboard as text, or writes every
//demi:describe item to a directory.
type ClipboardReadInput struct {
	Tabbed
	Format *ClipboardFormat `json:"format,omitzero"`
	// Output directory on the invoking Host
	OutputDir *string `json:"output-dir,omitzero" check:"chars=1..LocatorLength"`
	Overwrite *bool   `json:"overwrite,omitzero"`
	Timed
}

// A ClipboardItem is a clipboard item clipboard.read wrote to a file.
//
//demi:wire
//demi:describe A clipboard item `clipboard.read` wrote to a file.
type ClipboardItem struct {
	MimeType ClipboardMime `json:"mimeType"`
	Path     string        `json:"path"`
	Bytes    uint          `json:"bytes"`
}

// A ClipboardReadResult is what clipboard.read answers: the text, or the items
// it wrote to files.
//
//demi:union untagged
//demi:schema
type ClipboardReadResult interface {
	clipboardReadResult()
}

// ClipboardText is the clipboard's text.
//
//demi:variant
type ClipboardText struct {
	Text string `json:"text"`
}

// ClipboardItems are the items that were written to files.
//
//demi:variant
type ClipboardItems struct {
	Items []ClipboardItem `json:"items"`
}

func (ClipboardText) clipboardReadResult()  {}
func (ClipboardItems) clipboardReadResult() {}

// EvalInput is the input of browser.eval, which evaluates a read-only expression
// in the page, or a function of the target's element.
//
//demi:wire
//demi:schema
//demi:describe `eval`: evaluates a read-only expression in the page, or a function of
//demi:describe the target's element.
type EvalInput struct {
	Tabbed
	BrowserTarget
	Expression string `json:"expression" check:"chars=..StdinChars"`
	All        *bool  `json:"all,omitzero"`
	Timed
}

// EvalResult is what eval answers.
//
//demi:wire
//demi:schema
type EvalResult struct {
	Value jsontext.Value `json:"value"`
}

// LogsInput is the input of browser.logs, which reads a tab's console entries
// after a cursor.
//
//demi:wire
//demi:schema
//demi:describe `logs`: reads a tab's console entries after a cursor.
type LogsInput struct {
	Tabbed
	Level  *[]LogLevel `json:"level,omitzero"`
	Filter *string     `json:"filter,omitzero" check:"chars=..StdinChars"`
	// Cursor returned by a previous read
	After *string `json:"after,omitzero" check:"chars=1..LocatorLength"`
	Limit *uint   `json:"limit,omitzero" check:"range=1..MaxNodes"`
	Timed
}

// A LogEntry is one console entry, numbered in the tab's order.
//
//demi:wire
//demi:describe One console entry, numbered in the tab's order.
type LogEntry struct {
	Sequence uint64   `json:"sequence"`
	Level    LogLevel `json:"level"`
	Text     string   `json:"text"`
	URL      *string  `json:"url,omitzero"`
	// Milliseconds since the Unix epoch.
	Timestamp float64 `json:"timestamp"`
}

// LogsResult is what logs answers.
//
//demi:wire
//demi:schema
type LogsResult struct {
	Entries   []LogEntry `json:"entries"`
	Cursor    string     `json:"cursor"`
	HasMore   bool       `json:"hasMore"`
	Truncated bool       `json:"truncated"`
}

// ViewportSetInput is the input of browser.viewport.set, which gives a tab the
// agent's viewport.
//
//demi:wire
//demi:schema
//demi:describe `viewport.set`: gives a tab the agent's viewport.
type ViewportSetInput struct {
	Tabbed
	Width  uint32 `json:"width" check:"range=1..4096"`
	Height uint32 `json:"height" check:"range=1..4096"`
	// Device pixel ratio, 1 by default
	Scale *float64 `json:"scale,omitzero" check:"range=0.5..4.0"`
	Timed
}

// ViewportResetInput is the input of browser.viewport.reset, which returns a
// tab's viewport to the user's panel.
//
//demi:wire
//demi:schema
//demi:describe `viewport.reset`: returns a tab's viewport to the user's panel.
type ViewportResetInput struct {
	Tabbed
	Timed
}

// ViewportResult is what viewport.set and viewport.reset answer.
//
//demi:wire
//demi:schema
type ViewportResult struct {
	Viewport BrowserViewport `json:"viewport"`
}

// DialogInspectInput is the input of browser.dialog.inspect, which reads the
// tab's JavaScript dialog.
//
//demi:wire
//demi:schema
//demi:describe `dialog.inspect`: reads the tab's JavaScript dialog.
type DialogInspectInput struct {
	Tabbed
	Timed
}

// DialogInspectResult is what dialog.inspect answers: the dialog, or null when
// none is open.
//
//demi:wire
//demi:schema
//demi:describe What `dialog.inspect` answers: the dialog, or `null` when none is open.
type DialogInspectResult struct {
	Dialog *Dialog `json:"dialog" check:"nullable"`
}

// DialogAcceptInput is the input of browser.dialog.accept, which accepts the
// tab's dialog, answering a prompt with text.
//
//demi:wire
//demi:schema
//demi:describe `dialog.accept`: accepts the tab's dialog, answering a prompt with text.
type DialogAcceptInput struct {
	Tabbed
	Text *string `json:"text,omitzero" check:"chars=..StdinChars"`
	Timed
}

// DialogDismissInput is the input of browser.dialog.dismiss, which dismisses
// the tab's dialog.
//
//demi:wire
//demi:schema
//demi:describe `dialog.dismiss`: dismisses the tab's dialog.
type DialogDismissInput struct {
	Tabbed
	Timed
}

// DialogResult is what dialog.accept and dialog.dismiss answer.
//
//demi:wire
//demi:schema
type DialogResult struct {
	Type    DialogType    `json:"type"`
	Outcome DialogOutcome `json:"outcome"`
}

// CDPTargetsInput is the input of browser.cdp.targets, which lists the CDP
// targets of a tab: its page, frames and workers.
//
//demi:wire
//demi:schema
//demi:describe `cdp.targets`: lists the CDP targets of a tab: its page, frames and workers.
type CDPTargetsInput struct {
	Tabbed
	Offset *uint `json:"offset,omitzero"`
	Limit  *uint `json:"limit,omitzero" check:"range=1..MaxNodes"`
	Timed
}

// A CDPTarget is a CDP target of a tab.
//
//demi:wire
type CDPTarget struct {
	ID   string `json:"id"`
	Kind string `json:"kind"`
	URL  string `json:"url"`
}

// CDPTargetsResult is what cdp.targets answers.
//
//demi:wire
//demi:schema
type CDPTargetsResult struct {
	Targets   []CDPTarget `json:"targets"`
	Truncated bool        `json:"truncated"`
}

// CDPDetachInput is the input of browser.cdp.detach, which ends this caller's
// debugging connection to a tab.
//
//demi:wire
//demi:schema
//demi:describe `cdp.detach`: ends this caller's debugging connection to a tab.
type CDPDetachInput struct {
	Tabbed
	Timed
}

// CDPDetachResult is what cdp.detach answers.
//
//demi:wire
//demi:schema
type CDPDetachResult struct {
	Detached TabID `json:"detached"`
}

// CDPSendInput is the input of browser.cdp.send, which sends one CDP command
// with JSON parameters.
//
//demi:wire
//demi:schema
//demi:describe `cdp.send`: sends one CDP command with JSON parameters.
type CDPSendInput struct {
	Tabbed
	Method string  `json:"method" check:"chars=1..LocatorLength"`
	Params string  `json:"params" check:"chars=..StdinChars"`
	Target *string `json:"target,omitzero" check:"chars=1..LocatorLength"`
	Timed
}

// CDPSendResult is what cdp.send answers.
//
//demi:wire
//demi:schema
type CDPSendResult struct {
	Method string         `json:"method"`
	Result jsontext.Value `json:"result"`
}

// CDPEventsInput is the input of browser.cdp.events, which reads the CDP events
// this caller's connection received after a cursor.
//
//demi:wire
//demi:schema
//demi:describe `cdp.events`: reads the CDP events this caller's connection received
//demi:describe after a cursor.
type CDPEventsInput struct {
	Tabbed
	Method *[]string `json:"method,omitzero" check:"each(chars=1..LocatorLength)"`
	// Cursor returned by a previous read
	After  *string `json:"after,omitzero" check:"chars=1..LocatorLength"`
	Limit  *uint   `json:"limit,omitzero" check:"range=1..MaxNodes"`
	Target *string `json:"target,omitzero" check:"chars=1..LocatorLength"`
	Timed
}

// A CDPEvent is one CDP event a connection received.
//
//demi:wire
type CDPEvent struct {
	Sequence uint64         `json:"sequence"`
	Method   string         `json:"method"`
	Params   jsontext.Value `json:"params"`
	Target   string         `json:"target"`
}

// CDPEventsResult is what cdp.events answers.
//
//demi:wire
//demi:schema
type CDPEventsResult struct {
	Events    []CDPEvent `json:"events"`
	Cursor    string     `json:"cursor"`
	HasMore   bool       `json:"hasMore"`
	Truncated bool       `json:"truncated"`
}

// ContentReadInput is the input of browser.content.read, which reads a tab's
// content as text, HTML or DOM, inline or into a file.
//
//demi:wire
//demi:schema
//demi:describe `content.read`: reads a tab's content as text, HTML or DOM, inline or
//demi:describe into a file.
type ContentReadInput struct {
	Tabbed
	Format *ContentFormat `json:"format,omitzero"`
	// New output file on this Host
	Output    *string `json:"output,omitzero" check:"chars=1..LocatorLength"`
	Overwrite *bool   `json:"overwrite,omitzero"`
	Timed
}

// A ContentReadResult is what content.read answers: the content inline, or the
// file it was written to.
//
//demi:union untagged
//demi:schema
type ContentReadResult interface {
	contentReadResult()
}

// ContentInline is a page's content, inline.
//
//demi:variant
type ContentInline struct {
	URL       string        `json:"url"`
	Title     string        `json:"title"`
	Format    ContentFormat `json:"format"`
	Content   string        `json:"content"`
	Truncated bool          `json:"truncated"`
}

// ContentFile is a page's content, in a file.
//
//demi:variant
type ContentFile struct {
	URL    string        `json:"url"`
	Title  string        `json:"title"`
	Format ContentFormat `json:"format"`
	Path   string        `json:"path"`
}

func (ContentInline) contentReadResult() {}
func (ContentFile) contentReadResult()   {}

// ContentFetchInput is the input of browser.content.fetch, which loads URLs in
// temporary tabs and reads their content.
//
//demi:wire
//demi:schema
//demi:describe `content.fetch`: loads URLs in temporary tabs and reads their content.
type ContentFetchInput struct {
	URL    []string       `json:"url" check:"items=1..FetchURLs,each(chars=1..LocatorLength)"`
	Format *ContentFormat `json:"format,omitzero"`
	Timed
}

// A FetchedPage is one URL content.fetch read, or its failure.
//
//demi:wire
//demi:describe One URL `content.fetch` read, or its failure.
type FetchedPage struct {
	RequestedURL string          `json:"requestedUrl"`
	URL          string          `json:"url"`
	Title        string          `json:"title"`
	Content      string          `json:"content"`
	Error        *BrowserFailure `json:"error,omitzero"`
}

// ContentFetchResult is what content.fetch answers.
//
//demi:wire
//demi:schema
type ContentFetchResult struct {
	Pages     []FetchedPage `json:"pages"`
	Truncated bool          `json:"truncated"`
}

// AssetsListInput is the input of browser.assets.list, which lists a tab's
// fonts, images, stylesheets, videos and inline SVGs as an inventory.
//
//demi:wire
//demi:schema
//demi:describe `assets.list`: lists a tab's fonts, images, stylesheets, videos and
//demi:describe inline SVGs as an inventory.
type AssetsListInput struct {
	Tabbed
	Timed
}

// An Asset is a font, image, stylesheet or video a tab uses.
//
//demi:wire
type Asset struct {
	ID       string    `json:"id"`
	Kind     AssetKind `json:"kind"`
	URL      string    `json:"url"`
	MimeType *string   `json:"mimeType,omitzero"`
}

// An InlineSVG is an SVG element of a page.
//
//demi:wire
type InlineSVG struct {
	ID   string `json:"id"`
	HTML string `json:"html"`
}

// AssetsListResult is what assets.list answers.
//
//demi:wire
//demi:schema
type AssetsListResult struct {
	Inventory  string      `json:"inventory"`
	Assets     []Asset     `json:"assets"`
	InlineSVGs []InlineSVG `json:"inlineSvgs"`
	Truncated  bool        `json:"truncated"`
}

// AssetsExportInput is the input of browser.assets.export, which saves an
// inventory's assets to a directory with a manifest.
//
//demi:wire
//demi:schema
//demi:describe `assets.export`: saves an inventory's assets to a directory with a manifest.
type AssetsExportInput struct {
	Tabbed
	Inventory string       `json:"inventory" check:"chars=1..LocatorLength"`
	ID        *[]string    `json:"id,omitzero" check:"each(chars=1..LocatorLength)"`
	Kind      *[]AssetKind `json:"kind,omitzero"`
	// Output directory on the invoking Host
	OutputDir string `json:"output-dir" check:"chars=1..LocatorLength"`
	Overwrite *bool  `json:"overwrite,omitzero"`
	Timed
}

// An ExportedAsset is an asset assets.export saved.
//
//demi:wire
//demi:describe An asset `assets.export` saved.
type ExportedAsset struct {
	ID       string `json:"id"`
	Path     string `json:"path"`
	Bytes    uint   `json:"bytes"`
	MimeType string `json:"mimeType"`
}

// AssetsExportResult is what assets.export answers.
//
//demi:wire
//demi:schema
type AssetsExportResult struct {
	Directory string          `json:"directory"`
	Manifest  string          `json:"manifest"`
	Files     []ExportedAsset `json:"files"`
}

// CapabilitiesInput is the input of browser.capabilities, which lists what the
// page supports, such as WebMCP.
//
//demi:wire
//demi:schema
//demi:describe `capabilities`: lists what the page supports, such as WebMCP.
type CapabilitiesInput struct {
	Tabbed
	Timed
}

// A Capability is something the page supports, or why it does not.
//
//demi:wire
type Capability struct {
	ID        string          `json:"id"`
	Available bool            `json:"available"`
	Reason    *string         `json:"reason,omitzero"`
	Schema    *jsontext.Value `json:"schema,omitzero"`
}

// CapabilitiesResult is what capabilities answers.
//
//demi:wire
//demi:schema
type CapabilitiesResult struct {
	Capabilities []Capability `json:"capabilities"`
}

// WebmcpListInput is the input of browser.webmcp.list, which lists the WebMCP
// tools the page declares.
//
//demi:wire
//demi:schema
//demi:describe `webmcp.list`: lists the WebMCP tools the page declares.
type WebmcpListInput struct {
	Tabbed
	Timed
}

// A WebmcpTool is a tool the page declares.
//
//demi:wire
type WebmcpTool struct {
	Name         string          `json:"name"`
	Description  string          `json:"description"`
	InputSchema  jsontext.Value  `json:"inputSchema"`
	OutputSchema *jsontext.Value `json:"outputSchema,omitzero"`
}

// WebmcpListResult is what webmcp.list answers: the declarations' generation,
// which webmcp.call names, and the tools.
//
//demi:wire
//demi:schema
//demi:describe What `webmcp.list` answers: the declarations' generation, which
//demi:describe `webmcp.call` names, and the tools.
type WebmcpListResult struct {
	Tools     string       `json:"tools"`
	Entries   []WebmcpTool `json:"entries"`
	Truncated bool         `json:"truncated"`
}

// WebmcpCallInput is the input of browser.webmcp.call, which calls one of the
// page's WebMCP tools with JSON arguments.
//
//demi:wire
//demi:schema
//demi:describe `webmcp.call`: calls one of the page's WebMCP tools with JSON arguments.
type WebmcpCallInput struct {
	Tabbed
	Tool string `json:"tool" check:"chars=1..LocatorLength"`
	// The generation `webmcp.list` returned
	Tools     string `json:"tools" check:"chars=1..LocatorLength"`
	Arguments string `json:"arguments" check:"chars=..StdinChars"`
	Timed
}

// WebmcpCallResult is what webmcp.call answers.
//
//demi:wire
//demi:schema
type WebmcpCallResult struct {
	Name   string         `json:"name"`
	Result jsontext.Value `json:"result"`
}

// LiveInput is the input of browser.live, which serves a viewer of the
// conversation's browser: none; the page and the module speak over the stream.
//
//demi:wire
//demi:describe The view's arguments: none; the page and the module speak over the stream.
type LiveInput struct{}
