package browserop

import "encoding/json"

//go:generate go run github.com/wspl/demi/tools/contractgen

// `open`: opens a tab at a URL, starting the browser when it does not run.
// +demi:root
// +demi:schema
type OpenInput struct {
	// +demi:length chars min=1 max=4096
	URL  string `json:"url"`
	Load *Load  `json:"load,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `tabs`: lists the browser's tabs.
// +demi:root
// +demi:schema
type TabsInput struct {
	Offset *uint `json:"offset,omitempty"`
	// +demi:range min=1 max=1000
	Limit *uint `json:"limit,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `info`: reads a tab's URL, title, viewport and dialog.
// +demi:root
// +demi:schema
type InfoInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `goto`: navigates a tab to a URL.
// +demi:root
// +demi:schema
type GotoInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// +demi:length chars min=1 max=4096
	URL  string `json:"url"`
	Load *Load  `json:"load,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `back`: goes one entry back in a tab's history.
// +demi:root
// +demi:schema
type BackInput struct {
	// Browser tab ID returned by open or tabs
	Tab  TabID `json:"tab"`
	Load *Load `json:"load,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `forward`: goes one entry forward in a tab's history.
// +demi:root
// +demi:schema
type ForwardInput struct {
	// Browser tab ID returned by open or tabs
	Tab  TabID `json:"tab"`
	Load *Load `json:"load,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `reload`: reloads a tab's document.
// +demi:root
// +demi:schema
type ReloadInput struct {
	// Browser tab ID returned by open or tabs
	Tab  TabID `json:"tab"`
	Load *Load `json:"load,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `history`: lists a tab's navigation entries.
// +demi:root
// +demi:schema
type HistoryInput struct {
	// Browser tab ID returned by open or tabs
	Tab    TabID `json:"tab"`
	Offset *uint `json:"offset,omitempty"`
	// +demi:range min=1 max=1000
	Limit *uint `json:"limit,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `close`: closes a tab.
// +demi:root
// +demi:schema
type CloseInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `inspect`: reads a tab's accessibility or DOM tree.
// +demi:root
// +demi:schema
type InspectInput struct {
	// Browser tab ID returned by open or tabs
	Tab  TabID        `json:"tab"`
	View *InspectView `json:"view,omitempty"`
	// Container node reference
	Within *NodeRef `json:"within,omitempty"`
	// Frame references, outermost to innermost
	Frame *[]NodeRef `json:"frame,omitempty"`
	// +demi:range min=1 max=1000
	Limit *uint `json:"limit,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `find`: lists the elements a target or a query tree matches.
// +demi:root
// +demi:schema
type FindInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	Offset *uint `json:"offset,omitempty"`
	// +demi:range min=1 max=1000
	Limit *uint `json:"limit,omitempty"`
	// Read a declarative query tree from stdin
	Query *bool `json:"query,omitempty"`
	// JSON query tree when --query is supplied
	// +demi:length chars max=1048576
	Body *string `json:"body,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `read`: reads a property or an attribute of the target's element, or
// of every match with `--all`.
// +demi:root
// +demi:schema
type ReadInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	Property *ReadProperty `json:"property,omitempty"`
	// +demi:length chars min=1 max=4096
	Attribute *string `json:"attribute,omitempty"`
	All       *bool   `json:"all,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `screenshot`: captures a tab's viewport, whole page or a rectangle as PNG.
// +demi:root
// +demi:schema
type ScreenshotInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// New output file on this Host
	// +demi:length chars min=1 max=4096
	Output    *string `json:"output,omitempty"`
	Overwrite *bool   `json:"overwrite,omitempty"`
	FullPage  *bool   `json:"full-page,omitempty"`
	// CSS rectangle: x,y,width,height
	// +demi:length chars min=1 max=4096
	Clip *string `json:"clip,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `probe`: lists the nodes under a viewport point.
// +demi:root
// +demi:schema
type ProbeInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Viewport CSS coordinates: x,y
	// +demi:length chars min=1 max=4096
	XY                     string `json:"xy"`
	IncludeNonInteractable *bool  `json:"include-non-interactable,omitempty"`
	// New output file on this Host
	// +demi:length chars min=1 max=4096
	Output    *string `json:"output,omitempty"`
	Overwrite *bool   `json:"overwrite,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `click`: clicks the target's element or a viewport point.
// +demi:root
// +demi:schema
type ClickInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// Viewport CSS coordinates: x,y
	// +demi:length chars min=1 max=4096
	XY       *string     `json:"xy,omitempty"`
	Modifier *[]Modifier `json:"modifier,omitempty"`
	// +demi:range min=1 max=2
	Count  *uint8       `json:"count,omitempty"`
	Button *MouseButton `json:"button,omitempty"`
	// Expected URL glob after the action
	// +demi:length chars min=1 max=4096
	WaitURL *string `json:"wait-url,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `move`: moves the pointer over the target's element or to a point.
// +demi:root
// +demi:schema
type MoveInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// Viewport CSS coordinates: x,y
	// +demi:length chars min=1 max=4096
	XY       *string     `json:"xy,omitempty"`
	Modifier *[]Modifier `json:"modifier,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `drag`: drags the pointer through viewport points.
// +demi:root
// +demi:schema
type DragInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Viewport CSS coordinates x,y, at least two
	// +demi:length min=2
	Point    []LocatorText `json:"point"`
	Modifier *[]Modifier   `json:"modifier,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `scroll`: scrolls at the target's element or a point.
// +demi:root
// +demi:schema
type ScrollInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// Viewport CSS coordinates: x,y
	// +demi:length chars min=1 max=4096
	XY       *string     `json:"xy,omitempty"`
	Modifier *[]Modifier `json:"modifier,omitempty"`
	Dx       *float64    `json:"dx,omitempty"`
	Dy       *float64    `json:"dy,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `fill`: replaces the value of the target's field.
// +demi:root
// +demi:schema
type FillInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// +demi:length chars max=1048576
	Text string `json:"text"`
	// Expected URL glob after the action
	// +demi:length chars min=1 max=4096
	WaitURL *string `json:"wait-url,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `type`: types text key by key, into the target or the focused element.
// +demi:root
// +demi:schema
type TypeInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// +demi:length chars max=1048576
	Text string `json:"text"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `key`: presses a key or a chord.
// +demi:root
// +demi:schema
type KeyInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// +demi:length chars min=1 max=4096
	Key string `json:"key"`
	// Expected URL glob after the action
	// +demi:length chars min=1 max=4096
	WaitURL *string `json:"wait-url,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `check`: checks or unchecks the target's checkbox or radio button.
// +demi:root
// +demi:schema
type CheckInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	Value bool `json:"value"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `select`: selects options of the target's select element by value,
// label or index.
// +demi:root
// +demi:schema
// +demi:check validateSelectInput
type SelectInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	Value       *[]string `json:"value,omitempty"`
	OptionLabel *[]string `json:"option-label,omitempty"`
	OptionIndex *[]uint   `json:"option-index,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `select-text`: selects text inside the target, or places the cursor
// before or after it.
// +demi:root
// +demi:schema
type SelectTextInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// +demi:length chars max=1048576
	Text   string      `json:"text"`
	Cursor *TextCursor `json:"cursor,omitempty"`
	// +demi:length chars max=1048576
	Prefix *string `json:"prefix,omitempty"`
	// +demi:length chars max=1048576
	Suffix *string `json:"suffix,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `wait`: waits for a URL, the current document's load, or an element
// condition.
// +demi:root
// +demi:schema
type WaitInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// +demi:length chars min=1 max=4096
	URL   *string       `json:"url,omitempty"`
	Load  *Load         `json:"load,omitempty"`
	State *ElementState `json:"state,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `upload`: sets the files of the target's file input.
// +demi:root
// +demi:schema
type UploadInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// +demi:length min=1
	File []LocatorText `json:"file"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `download`: clicks the target or a point and saves the download it starts.
// +demi:root
// +demi:schema
type DownloadInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// Viewport CSS coordinates: x,y
	// +demi:length chars min=1 max=4096
	XY       *string     `json:"xy,omitempty"`
	Modifier *[]Modifier `json:"modifier,omitempty"`
	// New output file on this Host
	// +demi:length chars min=1 max=4096
	Output    *string `json:"output,omitempty"`
	Overwrite *bool   `json:"overwrite,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `clipboard.write`: writes stdin to the tab's clipboard.
// +demi:root
// +demi:schema
type ClipboardWriteInput struct {
	// Browser tab ID returned by open or tabs
	Tab  TabID          `json:"tab"`
	MIME *ClipboardMime `json:"mime,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `clipboard.read`: reads the tab's clipboard as text, or writes every
// item to a directory.
// +demi:root
// +demi:schema
type ClipboardReadInput struct {
	// Browser tab ID returned by open or tabs
	Tab    TabID            `json:"tab"`
	Format *ClipboardFormat `json:"format,omitempty"`
	// Output directory on the invoking Host
	// +demi:length chars min=1 max=4096
	OutputDir *string `json:"output-dir,omitempty"`
	Overwrite *bool   `json:"overwrite,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `eval`: evaluates a read-only expression in the page, or a function of
// the target's element.
// +demi:root
// +demi:schema
type EvalInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	BrowserTarget
	// +demi:length chars max=1048576
	Expression string `json:"expression"`
	All        *bool  `json:"all,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `logs`: reads a tab's console entries after a cursor.
// +demi:root
// +demi:schema
type LogsInput struct {
	// Browser tab ID returned by open or tabs
	Tab   TabID       `json:"tab"`
	Level *[]LogLevel `json:"level,omitempty"`
	// +demi:length chars max=1048576
	Filter *string `json:"filter,omitempty"`
	// Cursor returned by a previous read
	// +demi:length chars min=1 max=4096
	After *string `json:"after,omitempty"`
	// +demi:range min=1 max=1000
	Limit *uint `json:"limit,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `viewport.set`: gives a tab the agent's viewport.
// +demi:root
// +demi:schema
type ViewportSetInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// +demi:range min=1 max=4096
	Width uint32 `json:"width"`
	// +demi:range min=1 max=4096
	Height uint32 `json:"height"`
	// Device pixel ratio, 1 by default
	// +demi:range min=0.5 max=4.0
	Scale *float64 `json:"scale,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `viewport.reset`: returns a tab's viewport to the user's panel.
// +demi:root
// +demi:schema
type ViewportResetInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `dialog.inspect`: reads the tab's JavaScript dialog.
// +demi:root
// +demi:schema
type DialogInspectInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `dialog.accept`: accepts the tab's dialog, answering a prompt with text.
// +demi:root
// +demi:schema
type DialogAcceptInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// +demi:length chars max=1048576
	Text *string `json:"text,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `dialog.dismiss`: dismisses the tab's dialog.
// +demi:root
// +demi:schema
type DialogDismissInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `cdp.targets`: lists the CDP targets of a tab: its page, frames and workers.
// +demi:root
// +demi:schema
type CdpTargetsInput struct {
	// Browser tab ID returned by open or tabs
	Tab    TabID `json:"tab"`
	Offset *uint `json:"offset,omitempty"`
	// +demi:range min=1 max=1000
	Limit *uint `json:"limit,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `cdp.detach`: ends this caller's debugging connection to a tab.
// +demi:root
// +demi:schema
type CdpDetachInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `cdp.send`: sends one CDP command with JSON parameters.
// +demi:root
// +demi:schema
type CdpSendInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// +demi:length chars min=1 max=4096
	Method string `json:"method"`
	// +demi:length chars max=1048576
	Params string `json:"params"`
	// +demi:length chars min=1 max=4096
	Target *string `json:"target,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `cdp.events`: reads the CDP events this caller's connection received
// after a cursor.
// +demi:root
// +demi:schema
// +demi:check validateCdpEventsInput
type CdpEventsInput struct {
	// Browser tab ID returned by open or tabs
	Tab    TabID     `json:"tab"`
	Method *[]string `json:"method,omitempty"`
	// Cursor returned by a previous read
	// +demi:length chars min=1 max=4096
	After *string `json:"after,omitempty"`
	// +demi:range min=1 max=1000
	Limit *uint `json:"limit,omitempty"`
	// +demi:length chars min=1 max=4096
	Target *string `json:"target,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `content.read`: reads a tab's content as text, HTML or DOM, inline or
// into a file.
// +demi:root
// +demi:schema
type ContentReadInput struct {
	// Browser tab ID returned by open or tabs
	Tab    TabID          `json:"tab"`
	Format *ContentFormat `json:"format,omitempty"`
	// New output file on this Host
	// +demi:length chars min=1 max=4096
	Output    *string `json:"output,omitempty"`
	Overwrite *bool   `json:"overwrite,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `content.fetch`: loads URLs in temporary tabs and reads their content.
// +demi:root
// +demi:schema
type ContentFetchInput struct {
	// +demi:length min=1 max=10
	URL    []LocatorText  `json:"url"`
	Format *ContentFormat `json:"format,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `assets.list`: lists a tab's fonts, images, stylesheets, videos and
// inline SVGs as an inventory.
// +demi:root
// +demi:schema
type AssetsListInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `assets.export`: saves an inventory's assets to a directory with a manifest.
// +demi:root
// +demi:schema
// +demi:check validateAssetsExportInput
type AssetsExportInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// +demi:length chars min=1 max=4096
	Inventory string       `json:"inventory"`
	ID        *[]string    `json:"id,omitempty"`
	Kind      *[]AssetKind `json:"kind,omitempty"`
	// Output directory on the invoking Host
	// +demi:length chars min=1 max=4096
	OutputDir string `json:"output-dir"`
	Overwrite *bool  `json:"overwrite,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `capabilities`: lists what the page supports, such as WebMCP.
// +demi:root
// +demi:schema
type CapabilitiesInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `webmcp.list`: lists the WebMCP tools the page declares.
// +demi:root
// +demi:schema
type WebmcpListInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// `webmcp.call`: calls one of the page's WebMCP tools with JSON arguments.
// +demi:root
// +demi:schema
type WebmcpCallInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// +demi:length chars min=1 max=4096
	Tool string `json:"tool"`
	// The generation `webmcp.list` returned
	// +demi:length chars min=1 max=4096
	Tools string `json:"tools"`
	// +demi:length chars max=1048576
	Arguments string `json:"arguments"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	TimeoutMS *uint64 `json:"timeout,omitempty"`
}

// How far a navigation loads before it answers.
// +demi:root
// +demi:enum commit domcontentloaded load
type Load string

// Wire values for Load.
const (
	LoadCommit           Load = "commit"
	LoadDomContentLoaded Load = "domcontentloaded"
	LoadLoad             Load = "load"
)

// Which tree `inspect` returns.
// +demi:root
// +demi:enum accessibility dom
type InspectView string

// Wire values for InspectView.
const (
	InspectViewAccessibility InspectView = "accessibility"
	InspectViewDom           InspectView = "dom"
)

// What `read` reads from each element; `--attribute` reads an attribute instead.
// +demi:root
// +demi:enum text text-content html value visible enabled checked
type ReadProperty string

// Wire values for ReadProperty.
const (
	ReadPropertyText        ReadProperty = "text"
	ReadPropertyTextContent ReadProperty = "text-content"
	ReadPropertyHTML        ReadProperty = "html"
	ReadPropertyValue       ReadProperty = "value"
	ReadPropertyVisible     ReadProperty = "visible"
	ReadPropertyEnabled     ReadProperty = "enabled"
	ReadPropertyChecked     ReadProperty = "checked"
)

// A modifier key held during pointer input. `ControlOrMeta` is Meta on
// macOS and Control elsewhere; the variant has no doc of its own, which
// would make the set's JSON Schema a union.
// +demi:root
// +demi:enum Alt Control ControlOrMeta Meta Shift
type Modifier string

// Wire values for Modifier.
const (
	ModifierAlt           Modifier = "Alt"
	ModifierControl       Modifier = "Control"
	ModifierControlOrMeta Modifier = "ControlOrMeta"
	ModifierMeta          Modifier = "Meta"
	ModifierShift         Modifier = "Shift"
)

// +demi:root
// +demi:enum left middle right
type MouseButton string

// Wire values for MouseButton.
const (
	MouseButtonLeft   MouseButton = "left"
	MouseButtonMiddle MouseButton = "middle"
	MouseButtonRight  MouseButton = "right"
)

// Where `select-text` leaves the cursor instead of selecting the text.
// +demi:root
// +demi:enum before after
type TextCursor string

// Wire values for TextCursor.
const (
	TextCursorBefore TextCursor = "before"
	TextCursorAfter  TextCursor = "after"
)

// The element condition `wait` waits for.
// +demi:root
// +demi:enum visible hidden attached detached enabled
type ElementState string

// Wire values for ElementState.
const (
	ElementStateVisible  ElementState = "visible"
	ElementStateHidden   ElementState = "hidden"
	ElementStateAttached ElementState = "attached"
	ElementStateDetached ElementState = "detached"
	ElementStateEnabled  ElementState = "enabled"
)

// A clipboard item's media type.
// +demi:root
// +demi:enum text/plain text/html image/png
type ClipboardMime string

// Wire values for ClipboardMime.
const (
	ClipboardMimeTextPlain ClipboardMime = "text/plain"
	ClipboardMimeTextHTML  ClipboardMime = "text/html"
	ClipboardMimeImagePng  ClipboardMime = "image/png"
)

// What `clipboard.read` returns inline: the text; without it, every item
// is written to files.
// +demi:root
// +demi:enum text
type ClipboardFormat string

// Wire values for ClipboardFormat.
const (
	ClipboardFormatText ClipboardFormat = "text"
)

// A console entry's level.
// +demi:root
// +demi:enum debug info log warning error
type LogLevel string

// Wire values for LogLevel.
const (
	LogLevelDebug   LogLevel = "debug"
	LogLevelInfo    LogLevel = "info"
	LogLevelLog     LogLevel = "log"
	LogLevelWarning LogLevel = "warning"
	LogLevelError   LogLevel = "error"
)

// The form a page's content takes: rendered text, HTML, or the DOM tree
// serialized.
// +demi:root
// +demi:enum text html dom
type ContentFormat string

// Wire values for ContentFormat.
const (
	ContentFormatText ContentFormat = "text"
	ContentFormatHTML ContentFormat = "html"
	ContentFormatDom  ContentFormat = "dom"
)

// +demi:root
// +demi:enum font image stylesheet video
type AssetKind string

// Wire values for AssetKind.
const (
	AssetKindFont       AssetKind = "font"
	AssetKindImage      AssetKind = "image"
	AssetKindStylesheet AssetKind = "stylesheet"
	AssetKindVideo      AssetKind = "video"
)

// +demi:root
// +demi:enum alert confirm prompt beforeunload
type DialogType string

// Wire values for DialogType.
const (
	DialogTypeAlert        DialogType = "alert"
	DialogTypeConfirm      DialogType = "confirm"
	DialogTypePrompt       DialogType = "prompt"
	DialogTypeBeforeUnload DialogType = "beforeunload"
)

// +demi:root
// +demi:enum accepted dismissed
type DialogOutcome string

// Wire values for DialogOutcome.
const (
	DialogOutcomeAccepted  DialogOutcome = "accepted"
	DialogOutcomeDismissed DialogOutcome = "dismissed"
)

// Who decides a tab's viewport (`live-view.md` § Modes): the user's
// panel, a phone, or the agent.
// +demi:root
// +demi:enum web mobile custom
type ViewportMode string

// Wire values for ViewportMode.
const (
	ViewportModeWeb    ViewportMode = "web"
	ViewportModeMobile ViewportMode = "mobile"
	ViewportModeCustom ViewportMode = "custom"
)

// A declarative query tree (`browser.md` § Queries): one base, `match`,
// `and` or `or`, narrowed by a container, a frame, what the element has or
// lacks, its text, its visibility and an index.
// +demi:root
type BrowserQuery struct {
	Match  *BrowserQueryMatch `json:"match,omitempty"`
	Within *BrowserQuery      `json:"within,omitempty"`
	Frame  *BrowserQuery      `json:"frame,omitempty"`
	// +demi:length min=1
	And *[]BrowserQuery `json:"and,omitempty"`
	// +demi:length min=1
	Or     *[]BrowserQuery `json:"or,omitempty"`
	Has    *BrowserQuery   `json:"has,omitempty"`
	HasNot *BrowserQuery   `json:"hasNot,omitempty"`
	// +demi:length chars max=1048576
	HasText *string `json:"hasText,omitempty"`
	// +demi:length chars max=1048576
	HasNotText *string `json:"hasNotText,omitempty"`
	Visible    *bool   `json:"visible,omitempty"`
	Nth        *uint   `json:"nth,omitempty"`
}

// A node's box in viewport CSS pixels.
// +demi:root
type Bounds struct {
	X      float64 `json:"x"`
	Y      float64 `json:"y"`
	Width  float64 `json:"width"`
	Height float64 `json:"height"`
}

// The element an action or a wait resolved its target to, as the page's
// accessibility tree names it, with the reference that names it from then
// on. `role` and `name` are empty for an element the tree leaves out.
// +demi:root
type ResolvedElement struct {
	Ref  NodeRef `json:"ref"`
	Role string  `json:"role"`
	Name string  `json:"name"`
}

// An option `select` left selected: its value and the label it shows.
// +demi:root
type SelectedOption struct {
	Value string `json:"value"`
	Label string `json:"label"`
}

// Who opened a tab: an agent, a page's `window.open`, a temporary command
// such as `content.fetch`, or the user.
// +demi:root
// +demi:union tag=kind
//
//sumtype:decl
type BrowserCreatedBy interface{ browserCreatedBy() }

// An agent, by its number in the conversation.
// +demi:variant agent
type BrowserCreatedByAgent struct {
	// +demi:range max=9007199254740991
	Number uint64 `json:"number"`
}

func (*BrowserCreatedByAgent) browserCreatedBy() {}

// +demi:variant page
type BrowserCreatedByPage struct {
	Opener TabID `json:"opener"`
}

func (*BrowserCreatedByPage) browserCreatedBy() {}

// A temporary command of the agent with this number.
// +demi:variant temporary
type BrowserCreatedByTemporary struct {
	// +demi:range max=9007199254740991
	Number uint64 `json:"number"`
}

func (*BrowserCreatedByTemporary) browserCreatedBy() {}

// +demi:variant user
type BrowserCreatedByUser struct{}

func (*BrowserCreatedByUser) browserCreatedBy() {}

// A tab as `tabs` lists it and the conversation browser's tab methods return it.
// +demi:root direction=receive output=plugin-browser
type BrowserTab struct {
	ID        TabID            `json:"id"`
	Title     string           `json:"title"`
	URL       string           `json:"url"`
	CreatedBy BrowserCreatedBy `json:"createdBy"`
}

// A tab's viewport (`live-view.md` § Modes): its CSS size, the pixel ratio
// it renders at, and who decides them.
// +demi:root
type BrowserViewport struct {
	// +demi:range min=1
	Width uint32 `json:"width"`
	// +demi:range min=1
	Height uint32 `json:"height"`
	// +demi:range min=2.2250738585072014e-308
	DevicePixelRatio float64      `json:"devicePixelRatio"`
	Mode             ViewportMode `json:"mode"`
}

// A JavaScript dialog a tab shows.
// +demi:root
type Dialog struct {
	Type    DialogType `json:"type"`
	Message string     `json:"message"`
}

// What `open` answers: the new tab, with its title and viewport when the
// page reported them in time.
// +demi:root
// +demi:schema
type OpenResult struct {
	Tab      TabID            `json:"tab"`
	URL      string           `json:"url"`
	Title    *string          `json:"title,omitempty"`
	Viewport *BrowserViewport `json:"viewport,omitempty"`
}

// +demi:root
// +demi:schema
type TabsResult struct {
	Tabs      []BrowserTab `json:"tabs"`
	Truncated bool         `json:"truncated"`
}

// +demi:root
// +demi:schema
type InfoResult struct {
	Tab      TabID           `json:"tab"`
	URL      string          `json:"url"`
	Title    string          `json:"title"`
	Viewport BrowserViewport `json:"viewport"`
	Dialog   *Dialog         `json:"dialog,omitempty"`
}

// What a navigation answers: the URL it observed, with the title when the
// same document reported it in time.
// +demi:root
// +demi:schema
type NavigationResult struct {
	Tab   TabID   `json:"tab"`
	URL   string  `json:"url"`
	Title *string `json:"title,omitempty"`
}

// +demi:root
type HistoryEntry struct {
	Index   uint   `json:"index"`
	URL     string `json:"url"`
	Title   string `json:"title"`
	Current bool   `json:"current"`
}

// +demi:root
// +demi:schema
type HistoryResult struct {
	Entries   []HistoryEntry `json:"entries"`
	Truncated bool           `json:"truncated"`
}

// +demi:root
// +demi:schema
type CloseResult struct {
	Closed TabID `json:"closed"`
}

// What `read` answers: one value, or every match's with `--all`.
// +demi:root
// +demi:schema
// +demi:union untagged
//
//sumtype:decl
type ReadResult interface{ readResult() }

// +demi:variant
type ReadResultOne struct {
	Value json.RawMessage `json:"value"`
}

func (*ReadResultOne) readResult() {}

// +demi:variant
type ReadResultAll struct {
	Values    []json.RawMessage `json:"values"`
	Truncated bool              `json:"truncated"`
}

func (*ReadResultAll) readResult() {}

// The media type of a screenshot.
// +demi:root
// +demi:enum image/png
type ImageMime string

// Wire values for ImageMime.
const (
	ImageMimePng ImageMime = "image/png"
)

// What `screenshot` answers when it writes a file; `width` and `height` are
// in CSS pixels.
// +demi:root
// +demi:schema
type ScreenshotResult struct {
	Path     string          `json:"path"`
	MIMEType ImageMime       `json:"mimeType"`
	Width    uint32          `json:"width"`
	Height   uint32          `json:"height"`
	Viewport BrowserViewport `json:"viewport"`
}

// What a pointer or form action answers: the operation and the element it
// acted on, its own result, and what it observed after: the URL, tabs the
// page opened, and a dialog.
// +demi:root
// +demi:schema
type ActionResult struct {
	Operation string `json:"operation"`
	// The element the action's target resolved to; for untargeted `type`
	// and `key`, the focused element, and none when the focus is the
	// document. An action at coordinates names none.
	Target *ResolvedElement `json:"target,omitempty"`
	// The operation's own outcome: for `select`, the options it left
	// selected (`SelectedOption`).
	Result     json.RawMessage `json:"result"`
	URL        *string         `json:"url,omitempty"`
	OpenedTabs *[]TabID        `json:"openedTabs,omitempty"`
	Dialog     *Dialog         `json:"dialog,omitempty"`
}

// What `wait` answers; a load wait carries neither `url` nor `target`.
// +demi:root
// +demi:schema
type WaitResult struct {
	Condition string  `json:"condition"`
	Matched   bool    `json:"matched"`
	URL       *string `json:"url,omitempty"`
	// The element whose state matched; none when no element is attached.
	Target *ResolvedElement `json:"target,omitempty"`
}

// +demi:root
// +demi:schema
type UploadResult struct {
	Files    []string `json:"files"`
	Attached uint     `json:"attached"`
}

// +demi:root
// +demi:schema
type DownloadResult struct {
	Path              string `json:"path"`
	SuggestedFilename string `json:"suggestedFilename"`
	Bytes             uint64 `json:"bytes"`
	MIMEType          string `json:"mimeType"`
}

// +demi:root
// +demi:schema
type ClipboardWriteResult struct {
	MIMEType ClipboardMime `json:"mimeType"`
	Bytes    uint          `json:"bytes"`
}

// A clipboard item `clipboard.read` wrote to a file.
// +demi:root
type ClipboardItem struct {
	MIMEType ClipboardMime `json:"mimeType"`
	Path     string        `json:"path"`
	Bytes    uint          `json:"bytes"`
}

// +demi:root
// +demi:schema
// +demi:union untagged
//
//sumtype:decl
type ClipboardReadResult interface{ clipboardReadResult() }

// +demi:variant
type ClipboardReadResultText struct {
	Text string `json:"text"`
}

func (*ClipboardReadResultText) clipboardReadResult() {}

// +demi:variant
type ClipboardReadResultItems struct {
	Items []ClipboardItem `json:"items"`
}

func (*ClipboardReadResultItems) clipboardReadResult() {}

// +demi:root
// +demi:schema
type EvalResult struct {
	Value json.RawMessage `json:"value"`
}

// One console entry, numbered in the tab's order.
// +demi:root
type LogEntry struct {
	Sequence uint64   `json:"sequence"`
	Level    LogLevel `json:"level"`
	Text     string   `json:"text"`
	URL      *string  `json:"url,omitempty"`
	// Milliseconds since the Unix epoch.
	Timestamp float64 `json:"timestamp"`
}

// +demi:root
// +demi:schema
type LogsResult struct {
	Entries   []LogEntry `json:"entries"`
	Cursor    string     `json:"cursor"`
	HasMore   bool       `json:"hasMore"`
	Truncated bool       `json:"truncated"`
}

// +demi:root
// +demi:schema
type ViewportResult struct {
	Viewport BrowserViewport `json:"viewport"`
}

// What `dialog.inspect` answers: the dialog, or `null` when none is open.
// +demi:root
// +demi:schema
type DialogInspectResult struct {
	// +demi:nullable
	Dialog *Dialog `json:"dialog"`
}

// +demi:root
// +demi:schema
type DialogResult struct {
	Type    DialogType    `json:"type"`
	Outcome DialogOutcome `json:"outcome"`
}

// +demi:root
type CdpTarget struct {
	ID   string `json:"id"`
	Kind string `json:"kind"`
	URL  string `json:"url"`
}

// +demi:root
// +demi:schema
type CdpTargetsResult struct {
	Targets   []CdpTarget `json:"targets"`
	Truncated bool        `json:"truncated"`
}

// +demi:root
// +demi:schema
type CdpDetachResult struct {
	Detached TabID `json:"detached"`
}

// +demi:root
// +demi:schema
type CdpSendResult struct {
	Method string          `json:"method"`
	Result json.RawMessage `json:"result"`
}

// +demi:root
type CdpEvent struct {
	Sequence uint64          `json:"sequence"`
	Method   string          `json:"method"`
	Params   json.RawMessage `json:"params"`
	Target   string          `json:"target"`
}

// +demi:root
// +demi:schema
type CdpEventsResult struct {
	Events    []CdpEvent `json:"events"`
	Cursor    string     `json:"cursor"`
	HasMore   bool       `json:"hasMore"`
	Truncated bool       `json:"truncated"`
}

// +demi:root
// +demi:schema
// +demi:union untagged
//
//sumtype:decl
type ContentReadResult interface{ contentReadResult() }

// +demi:variant
type ContentReadResultInline struct {
	URL       string        `json:"url"`
	Title     string        `json:"title"`
	Format    ContentFormat `json:"format"`
	Content   string        `json:"content"`
	Truncated bool          `json:"truncated"`
}

func (*ContentReadResultInline) contentReadResult() {}

// +demi:variant
type ContentReadResultFile struct {
	URL    string        `json:"url"`
	Title  string        `json:"title"`
	Format ContentFormat `json:"format"`
	Path   string        `json:"path"`
}

func (*ContentReadResultFile) contentReadResult() {}

// +demi:root
type Asset struct {
	ID       string    `json:"id"`
	Kind     AssetKind `json:"kind"`
	URL      string    `json:"url"`
	MIMEType *string   `json:"mimeType,omitempty"`
}

// +demi:root
type InlineSvg struct {
	ID   string `json:"id"`
	HTML string `json:"html"`
}

// +demi:root
// +demi:schema
type AssetsListResult struct {
	Inventory  string      `json:"inventory"`
	Assets     []Asset     `json:"assets"`
	InlineSvgs []InlineSvg `json:"inlineSvgs"`
	Truncated  bool        `json:"truncated"`
}

// An asset `assets.export` saved.
// +demi:root
type ExportedAsset struct {
	ID       string `json:"id"`
	Path     string `json:"path"`
	Bytes    uint   `json:"bytes"`
	MIMEType string `json:"mimeType"`
}

// +demi:root
// +demi:schema
type AssetsExportResult struct {
	Directory string          `json:"directory"`
	Manifest  string          `json:"manifest"`
	Files     []ExportedAsset `json:"files"`
}

// +demi:root
type Capability struct {
	ID        string           `json:"id"`
	Available bool             `json:"available"`
	Reason    *string          `json:"reason,omitempty"`
	Schema    *json.RawMessage `json:"schema,omitempty"`
}

// +demi:root
// +demi:schema
type CapabilitiesResult struct {
	Capabilities []Capability `json:"capabilities"`
}

// +demi:root
type WebmcpTool struct {
	Name         string           `json:"name"`
	Description  string           `json:"description"`
	InputSchema  json.RawMessage  `json:"inputSchema"`
	OutputSchema *json.RawMessage `json:"outputSchema,omitempty"`
}

// What `webmcp.list` answers: the declarations' generation, which
// `webmcp.call` names, and the tools.
// +demi:root
// +demi:schema
type WebmcpListResult struct {
	Tools     string       `json:"tools"`
	Entries   []WebmcpTool `json:"entries"`
	Truncated bool         `json:"truncated"`
}

// +demi:root
// +demi:schema
type WebmcpCallResult struct {
	Name   string          `json:"name"`
	Result json.RawMessage `json:"result"`
}

// Why an operation failed. Each cause keeps its own code.
// +demi:root
// +demi:enum invalid_input tab_not_found tab_busy stale_ref stale_cursor stale_inventory stale_tools target_not_found ambiguous_target not_actionable timeout dialog_blocked dialog_not_found invalid_dialog_action history_boundary navigation_failed protected_value side_effect_rejected unsupported_result unsupported_capability cdp_method_denied output_exists io_error result_too_large partial_failure driver_error browser_unavailable browser_lost cancelled outcome_unknown
type BrowserErrorCode string

// Wire values for BrowserErrorCode.
const (
	BrowserErrorCodeInvalidInput          BrowserErrorCode = "invalid_input"
	BrowserErrorCodeTabNotFound           BrowserErrorCode = "tab_not_found"
	BrowserErrorCodeTabBusy               BrowserErrorCode = "tab_busy"
	BrowserErrorCodeStaleRef              BrowserErrorCode = "stale_ref"
	BrowserErrorCodeStaleCursor           BrowserErrorCode = "stale_cursor"
	BrowserErrorCodeStaleInventory        BrowserErrorCode = "stale_inventory"
	BrowserErrorCodeStaleTools            BrowserErrorCode = "stale_tools"
	BrowserErrorCodeTargetNotFound        BrowserErrorCode = "target_not_found"
	BrowserErrorCodeAmbiguousTarget       BrowserErrorCode = "ambiguous_target"
	BrowserErrorCodeNotActionable         BrowserErrorCode = "not_actionable"
	BrowserErrorCodeTimeout               BrowserErrorCode = "timeout"
	BrowserErrorCodeDialogBlocked         BrowserErrorCode = "dialog_blocked"
	BrowserErrorCodeDialogNotFound        BrowserErrorCode = "dialog_not_found"
	BrowserErrorCodeInvalidDialogAction   BrowserErrorCode = "invalid_dialog_action"
	BrowserErrorCodeHistoryBoundary       BrowserErrorCode = "history_boundary"
	BrowserErrorCodeNavigationFailed      BrowserErrorCode = "navigation_failed"
	BrowserErrorCodeProtectedValue        BrowserErrorCode = "protected_value"
	BrowserErrorCodeSideEffectRejected    BrowserErrorCode = "side_effect_rejected"
	BrowserErrorCodeUnsupportedResult     BrowserErrorCode = "unsupported_result"
	BrowserErrorCodeUnsupportedCapability BrowserErrorCode = "unsupported_capability"
	BrowserErrorCodeCdpMethodDenied       BrowserErrorCode = "cdp_method_denied"
	BrowserErrorCodeOutputExists          BrowserErrorCode = "output_exists"
	BrowserErrorCodeIOError               BrowserErrorCode = "io_error"
	BrowserErrorCodeResultTooLarge        BrowserErrorCode = "result_too_large"
	BrowserErrorCodePartialFailure        BrowserErrorCode = "partial_failure"
	BrowserErrorCodeDriverError           BrowserErrorCode = "driver_error"
	BrowserErrorCodeBrowserUnavailable    BrowserErrorCode = "browser_unavailable"
	BrowserErrorCodeBrowserLost           BrowserErrorCode = "browser_lost"
	BrowserErrorCodeCancelled             BrowserErrorCode = "cancelled"
	BrowserErrorCodeOutcomeUnknown        BrowserErrorCode = "outcome_unknown"
)

// How far an action's input got when it failed: not delivered, delivered
// with a later wait failing, or lost with the connection after delivery
// began.
// +demi:root
// +demi:enum not_started completed unknown
type ActionProgress string

// Wire values for ActionProgress.
const (
	ActionProgressNotStarted ActionProgress = "not_started"
	ActionProgressCompleted  ActionProgress = "completed"
	ActionProgressUnknown    ActionProgress = "unknown"
)

// The view's arguments: none; the page and the module speak over the stream.
// +demi:root
type LiveInput struct{}

// The viewer's platform, which decides how its keys map on the Host.
// +demi:root
// +demi:enum mac windows linux other
type Platform string

// Wire values for Platform.
const (
	PlatformMac     Platform = "mac"
	PlatformWindows Platform = "windows"
	PlatformLinux   Platform = "linux"
	PlatformOther   Platform = "other"
)

// A viewport mode the page may choose; `custom` is the agent's.
// +demi:root
// +demi:enum web mobile
type ViewerMode string

// Wire values for ViewerMode.
const (
	ViewerModeWeb    ViewerMode = "web"
	ViewerModeMobile ViewerMode = "mobile"
)

// +demi:root
// +demi:enum move down up
type PointerAction string

// Wire values for PointerAction.
const (
	PointerActionMove PointerAction = "move"
	PointerActionDown PointerAction = "down"
	PointerActionUp   PointerAction = "up"
)

// +demi:root
// +demi:enum none left middle right
type PointerButton string

// Wire values for PointerButton.
const (
	PointerButtonNone   PointerButton = "none"
	PointerButtonLeft   PointerButton = "left"
	PointerButtonMiddle PointerButton = "middle"
	PointerButtonRight  PointerButton = "right"
)

// +demi:root
// +demi:enum down up
type KeyAction string

// Wire values for KeyAction.
const (
	KeyActionDown KeyAction = "down"
	KeyActionUp   KeyAction = "up"
)

// A native form control an observer reports (`live-view.md` § Input).
// +demi:root
// +demi:enum select date month week time datetime-local color suggestions file
type ControlKind string

// Wire values for ControlKind.
const (
	ControlKindSelect        ControlKind = "select"
	ControlKindDate          ControlKind = "date"
	ControlKindMonth         ControlKind = "month"
	ControlKindWeek          ControlKind = "week"
	ControlKindTime          ControlKind = "time"
	ControlKindDatetimeLocal ControlKind = "datetime-local"
	ControlKindColor         ControlKind = "color"
	ControlKindSuggestions   ControlKind = "suggestions"
	ControlKindFile          ControlKind = "file"
)

// Why the browser ended: it stopped, or the conversation released it.
// +demi:root
// +demi:enum browser_ended released
type EndReason string

// Wire values for EndReason.
const (
	EndReasonBrowserEnded EndReason = "browser_ended"
	EndReasonReleased     EndReason = "released"
)

// A file the viewer chose for an upload; its bytes follow as file frames.
// +demi:root
type UploadFile struct {
	// +demi:length chars min=1 max=255
	// +demi:pattern ^[^/\\\x00]+$
	Name string `json:"name"`
	// +demi:length chars max=200
	MIMEType string `json:"mimeType"`
	// +demi:range max=9007199254740991
	Size uint64 `json:"size"`
}

// An option of a native select or suggestion list.
// +demi:root
type LiveControlOption struct {
	// +demi:length chars max=2000
	Label string `json:"label"`
	// +demi:length chars max=2000
	Value string `json:"value"`
	// +demi:length chars max=2000
	Group    string `json:"group"`
	Disabled bool   `json:"disabled"`
	Hidden   bool   `json:"hidden"`
	Selected bool   `json:"selected"`
}

// A control's box in viewport CSS pixels.
// +demi:root
type ControlRect struct {
	X float64 `json:"x"`
	Y float64 `json:"y"`
	// +demi:range min=2.2250738585072014e-308
	Width float64 `json:"width"`
	// +demi:range min=2.2250738585072014e-308
	Height float64 `json:"height"`
}

// A native form control of the watched tab, which the page draws over the
// picture.
// +demi:root
type LiveControl struct {
	Token ControlToken `json:"token"`
	// +demi:range max=9007199254740991
	Revision uint64      `json:"revision"`
	Kind     ControlKind `json:"kind"`
	// +demi:length chars max=2000
	Label string `json:"label"`
	// +demi:length chars max=10000
	Value string `json:"value"`
	// +demi:length chars max=100
	Min string `json:"min"`
	// +demi:length chars max=100
	Max string `json:"max"`
	// +demi:length chars max=100
	Step string `json:"step"`
	// +demi:length chars max=1000
	Accept   string `json:"accept"`
	Multiple bool   `json:"multiple"`
	Disabled bool   `json:"disabled"`
	Required bool   `json:"required"`
	Size     uint32 `json:"size"`
	// +demi:length max=1000
	Options []LiveControlOption `json:"options"`
	Rect    ControlRect         `json:"rect"`
}

// A tab as the view lists it.
// +demi:root
type LiveTab struct {
	ID        TabID            `json:"id"`
	Title     string           `json:"title"`
	URL       string           `json:"url"`
	CreatedBy BrowserCreatedBy `json:"createdBy"`
	Viewport  BrowserViewport  `json:"viewport"`
	// Whether the browser loads the tab's top-level page.
	Loading bool `json:"loading"`
}

// The watched tab's JavaScript dialog.
// +demi:root
type LiveDialog struct {
	Type        DialogType `json:"type"`
	Message     string     `json:"message"`
	DefaultText string     `json:"defaultText"`
}

// What the module sends.
// +demi:schema
// +demi:root direction=receive output=plugin-browser
// +demi:union tag=type
//
//sumtype:decl
type LiveModuleMessage interface{ liveModuleMessage() }

// The browser's tabs and the one this viewer watches.
// +demi:variant state
type LiveModuleMessageState struct {
	Running bool      `json:"running"`
	Tabs    []LiveTab `json:"tabs"`
	// +demi:nullable
	Watched *TabID `json:"watched"`
}

func (*LiveModuleMessageState) liveModuleMessage() {}

// Video frames of this generation follow, starting with a key frame.
// +demi:variant stream
type LiveModuleMessageStream struct {
	Tab        TabID  `json:"tab"`
	Generation uint32 `json:"generation"`
	// +demi:range min=1
	Width uint32 `json:"width"`
	// +demi:range min=1
	Height uint32 `json:"height"`
}

func (*LiveModuleMessageStream) liveModuleMessage() {}

// +demi:variant heartbeat
type LiveModuleMessageHeartbeat struct{}

func (*LiveModuleMessageHeartbeat) liveModuleMessage() {}

// +demi:variant cursor
type LiveModuleMessageCursor struct {
	Tab TabID `json:"tab"`
	// +demi:length chars max=200
	Cursor   string `json:"cursor"`
	Editable bool   `json:"editable"`
}

func (*LiveModuleMessageCursor) liveModuleMessage() {}

// +demi:variant controls
type LiveModuleMessageControls struct {
	Tab TabID `json:"tab"`
	// +demi:length max=100
	Controls []LiveControl `json:"controls"`
}

func (*LiveModuleMessageControls) liveModuleMessage() {}

// Text the watched tab copied shortly after this viewer's input.
// +demi:variant clipboard
type LiveModuleMessageClipboard struct {
	// +demi:length chars max=1000000
	Text string `json:"text"`
}

func (*LiveModuleMessageClipboard) liveModuleMessage() {}

// +demi:variant dialog
type LiveModuleMessageDialog struct {
	Tab TabID `json:"tab"`
	// +demi:nullable
	Dialog *LiveDialog `json:"dialog"`
}

func (*LiveModuleMessageDialog) liveModuleMessage() {}

// Whether a choice or an upload reached its control.
// +demi:variant choice
type LiveModuleMessageChoice struct {
	Token    ControlToken `json:"token"`
	Accepted bool         `json:"accepted"`
}

func (*LiveModuleMessageChoice) liveModuleMessage() {}

// Something the viewer asked for failed; the stream goes on.
// +demi:variant notice
type LiveModuleMessageNotice struct {
	// A browser failure's code, `capture_unavailable` or `capture_failed`.
	Code    string `json:"code"`
	Message string `json:"message"`
}

func (*LiveModuleMessageNotice) liveModuleMessage() {}

// The browser ended; the stream ends after this.
// +demi:variant ended
type LiveModuleMessageEnded struct {
	Reason EndReason `json:"reason"`
}

func (*LiveModuleMessageEnded) liveModuleMessage() {}

// What the module tells the extension.
// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type CaptureCommand interface{ captureCommand() }

// Capture the tab of CDP target `target` at `width` × `height` pixels.
// +demi:variant start
type CaptureCommandStart struct {
	Capture uint32 `json:"capture"`
	Target  string `json:"target"`
	Width   uint32 `json:"width"`
	Height  uint32 `json:"height"`
	FPS     uint32 `json:"fps"`
	Bitrate uint32 `json:"bitrate"`
}

func (*CaptureCommandStart) captureCommand() {}

// The consumer has the frames up to `sequence`; up to `window` more may
// be in flight.
// +demi:variant ack
type CaptureCommandAck struct {
	Capture  uint32 `json:"capture"`
	Sequence uint32 `json:"sequence"`
	Window   uint32 `json:"window"`
}

func (*CaptureCommandAck) captureCommand() {}

// Encode the next frame as a key frame.
// +demi:variant keyframe
type CaptureCommandKeyframe struct {
	Capture uint32 `json:"capture"`
}

func (*CaptureCommandKeyframe) captureCommand() {}

// +demi:variant encoding
type CaptureCommandEncoding struct {
	Capture uint32 `json:"capture"`
	Bitrate uint32 `json:"bitrate"`
	FPS     uint32 `json:"fps"`
}

func (*CaptureCommandEncoding) captureCommand() {}

// +demi:variant stop
type CaptureCommandStop struct {
	Capture uint32 `json:"capture"`
}

func (*CaptureCommandStop) captureCommand() {}

// What the extension tells the module.
// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type CaptureEvent interface{ captureEvent() }

// The connection is open and takes commands.
// +demi:variant ready
type CaptureEventReady struct{}

func (*CaptureEventReady) captureEvent() {}

// The capture's first picture is on its way.
// +demi:variant started
type CaptureEventStarted struct {
	Capture uint32 `json:"capture"`
}

func (*CaptureEventStarted) captureEvent() {}

// No picture arrived: the page stopped painting before capture began.
// +demi:variant stalled
type CaptureEventStalled struct {
	Capture uint32 `json:"capture"`
}

func (*CaptureEventStalled) captureEvent() {}

// The capture ended as the module asked.
// +demi:variant stopped
type CaptureEventStopped struct {
	Capture uint32 `json:"capture"`
}

func (*CaptureEventStopped) captureEvent() {}

// The capture failed and the extension released it.
// +demi:variant error
type CaptureEventError struct {
	Capture uint32 `json:"capture"`
	Message string `json:"message"`
}

func (*CaptureEventError) captureEvent() {}

// One Chrome version and its archive for each platform.
// +demi:root
type BrowserRelease struct {
	// Chrome's four-part version, such as `153.0.8010.36`.
	// +demi:pattern ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$
	Version   string            `json:"version"`
	Platforms []ReleasePlatform `json:"platforms"`
}

// One platform's archive.
// +demi:root
// +demi:check validateReleasePlatform
type ReleasePlatform struct {
	// The target triple the archive runs on, such as `x86_64-unknown-linux-musl`.
	// +demi:length min=1
	Target string `json:"target"`
	URL    string `json:"url"`
	// +demi:range min=1
	Size uint64 `json:"size"`
	// +demi:pattern ^[a-f0-9]{64}$
	SHA256 string `json:"sha256"`
	// The executable's path inside the archive.
	// +demi:length min=1
	Executable string `json:"executable"`
}

// An element target: a node reference, or a role, text, label,
// placeholder, test ID or CSS selector, narrowed by frames, a container
// and an index. Admission checks which fields may combine
// (`browser.md` § Element targets).
// +demi:root
type BrowserTarget struct {
	BrowserQueryMatch
	// Frame references, outermost to innermost
	Frame *[]NodeRef `json:"frame,omitempty"`
	// Explicit zero-based match index
	// +demi:range max=999
	Nth *uint `json:"nth,omitempty"`
	// Container node reference
	Within *NodeRef `json:"within,omitempty"`
}

// A query's base locator: a target without its frames, container and
// index, which the query's own `frame`, `within` and `nth` express.
// +demi:root
type BrowserQueryMatch struct {
	// A node reference returned by inspect or find
	Ref *NodeRef `json:"ref,omitempty"`
	// Accessible role, ASCII case-insensitive, such as button, textbox, or date
	// +demi:length chars min=1 max=4096
	Role *string `json:"role,omitempty"`
	// Accessible name, with --role
	// +demi:length chars min=1 max=4096
	Name *string `json:"name,omitempty"`
	// Accessible-name regular expression, with --role
	// +demi:length chars min=1 max=4096
	NamePattern *string `json:"name-pattern,omitempty"`
	// Rendered-text regular expression
	// +demi:length chars min=1 max=4096
	TextPattern *string `json:"text-pattern,omitempty"`
	// Associated label text (label for, wrapping label, or aria-labelledby)
	// +demi:length chars min=1 max=4096
	Label *string `json:"label,omitempty"`
	// +demi:length chars min=1 max=4096
	Placeholder *string `json:"placeholder,omitempty"`
	// Visible text to match
	// +demi:length chars min=1 max=4096
	TextMatch *string `json:"text-match,omitempty"`
	// data-testid attribute
	// +demi:length chars min=1 max=4096
	TestID *string `json:"test-id,omitempty"`
	// CSS selector
	// +demi:length chars min=1 max=4096
	CSS *string `json:"css,omitempty"`
	// Match the complete name or text
	Exact *bool `json:"exact,omitempty"`
}

// A tab's public identity, which `open` and `tabs` return: `t` and the
// tab's number in the conversation (`browser.md` § One tab registry).
// +demi:root
// +demi:id pattern=^t[1-9][0-9]{0,14}$
type TabID string

// A node reference, which `inspect`, `find` and `probe` return: `e` and
// a number unique within its tab. It stays valid while its document
// does.
// +demi:root
// +demi:id pattern=^e[1-9][0-9]{0,14}$
type NodeRef string

// A control's identity in its page, as `crypto.randomUUID` makes it.
// +demi:root
// +demi:id pattern=^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$
type ControlToken string

// +demi:length chars min=1 max=4096
type LocatorText string
