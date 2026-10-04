// Package schemas declares command-like contracts for the generator's JSON Schema tests.
package schemas

import "encoding/json"

//go:generate go run ../..

// `file.read`: writes the file's bytes to stdout.
// +demi:root
// +demi:schema
type ReadArgs struct {
	// File path to read
	Path string `json:"path"`
}

// `file.create`: creates a new file; an existing file is left as it is.
// +demi:root
// +demi:schema
type CreateArgs struct {
	// Target file path
	Path string `json:"path"`
	// File content
	Content string `json:"content"`
}

// `file.edit`: replaces one occurrence of exact text in an existing file.
// +demi:root
// +demi:schema
type EditArgs struct {
	// Target file path
	Path string `json:"path"`
	// Exact text to replace
	// +demi:length chars min=1
	Old string `json:"old"`
	// Replacement text
	New string `json:"new"`
	// 1-based occurrence to replace
	// +demi:range min=1
	Occurrence *uint `json:"occurrence,omitempty"`
	// Line number used to choose the nearest occurrence
	// +demi:range min=1
	Context *uint `json:"context,omitempty"`
}

// `file.patch`: applies a unified diff to one or more files.
// +demi:root
// +demi:schema
type PatchArgs struct {
	// Unified diff content
	Patch string `json:"patch"`
}

// How far a navigation loads before it answers.
// +demi:enum commit domcontentloaded load
type Load string

// A tab's public identity, which `open` and `tabs` return: `t` and the
// tab's number in the conversation (`browser.md` § One tab registry).
// +demi:id pattern=^t[1-9][0-9]{0,14}$
type TabID string

// `open`: opens a tab at a URL, starting the browser when it does not run.
// +demi:root
// +demi:schema
type OpenInput struct {
	// +demi:length chars min=1 max=4096
	URL  string `json:"url"`
	Load *Load  `json:"load,omitempty"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	Timeout *uint64 `json:"timeout,omitempty"`
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
	Timeout *uint64 `json:"timeout,omitempty"`
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
	Timeout *uint64 `json:"timeout,omitempty"`
}

// `forward`: goes one entry forward in a tab's history.
// +demi:root
// +demi:schema
type ForwardInput BackInput

// `reload`: reloads a tab's document.
// +demi:root
// +demi:schema
type ReloadInput BackInput

// What a navigation answers: the URL it observed, with the title when the
// same document reported it in time.
// +demi:root
// +demi:schema
type NavigationResult struct {
	Tab   TabID   `json:"tab"`
	URL   string  `json:"url"`
	Title *string `json:"title,omitempty"`
}

// `close`: closes a tab.
// +demi:root
// +demi:schema
type CloseInput struct {
	// Browser tab ID returned by open or tabs
	Tab TabID `json:"tab"`
	// Whole operation deadline in milliseconds
	// +demi:range min=1 max=300000
	Timeout *uint64 `json:"timeout,omitempty"`
}

// +demi:root
// +demi:schema
type CloseResult struct {
	Closed TabID `json:"closed"`
}

// Who decides a tab's viewport (`live-view.md` § Modes): the user's
// panel, a phone, or the agent.
// +demi:enum web mobile custom
type ViewportMode string

// A tab's viewport (`live-view.md` § Modes): its CSS size, the pixel ratio
// it renders at, and who decides them.
type BrowserViewport struct {
	// +demi:range min=1
	Width uint32 `json:"width"`
	// +demi:range min=1
	Height uint32 `json:"height"`
	// +demi:range min=2.2250738585072014e-308
	DevicePixelRatio float64      `json:"devicePixelRatio"`
	Mode             ViewportMode `json:"mode"`
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
	Timeout *uint64 `json:"timeout,omitempty"`
}

// `viewport.reset`: returns a tab's viewport to the user's panel.
// +demi:root
// +demi:schema
type ViewportResetInput CloseInput

// +demi:root
// +demi:schema
type ViewportResult struct {
	Viewport BrowserViewport `json:"viewport"`
}

// `cdp.detach`: ends this caller's debugging connection to a tab.
// +demi:root
// +demi:schema
type CdpDetachInput CloseInput

// +demi:root
// +demi:schema
type CdpDetachResult struct {
	Detached TabID `json:"detached"`
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
	Timeout *uint64 `json:"timeout,omitempty"`
}

// +demi:root
// +demi:schema
type WebmcpCallResult struct {
	Name   string          `json:"name"`
	Result json.RawMessage `json:"result"`
}

// An expose as the commands print it: by its number, never by its id,
// which is the URL's credential.
// +demi:tolerant
type ExposeLine struct {
	Number uint64 `json:"number"`
	// The device's name.
	Device  string `json:"device"`
	Address string `json:"address"`
	URL     string `json:"url"`
	// +demi:timestamp
	ExpiresAt string `json:"expiresAt"`
}

// `{ expose }`: what `add` and `renew` print with `--json`.
// +demi:root
// +demi:schema
// +demi:tolerant
type ExposeAnswer struct {
	Expose ExposeLine `json:"expose"`
}

// `{ exposes }`: what `list` prints with `--json`.
// +demi:root
// +demi:schema
// +demi:tolerant
type ExposeLines struct {
	Exposes []ExposeLine `json:"exposes"`
}

// The input of `demi example`.
// +demi:root
// +demi:schema
type ExampleArgs struct {
	// The file to read
	Path string `json:"path"`
	// How many times
	// +demi:range min=1 max=9
	Count   *uint32   `json:"count,omitempty"`
	Mode    *Mode     `json:"mode,omitempty"`
	Tags    []string  `json:"tags"`
	Labels  *[]string `json:"labels,omitempty"`
	NoCache *bool     `json:"no-cache,omitempty"`
}

// +demi:enum fast slow
type Mode string

// +demi:root
// +demi:schema
// +demi:union tag=kind
type Outcome interface{ outcome() }

// +demi:variant Outcome ok
type Success struct {
	// +demi:nullable
	Value *string `json:"value"`
}

// +demi:variant Outcome error
type Failure struct {
	Message string `json:"message"`
}

// +demi:root
// +demi:schema
// +demi:tolerant
type Collection struct {
	// +demi:length min=1 max=2
	Values []Small            `json:"values"`
	Labels map[string]*string `json:"labels"`
}

// +demi:range min=1 max=8
type Small uint8

// +demi:root
// +demi:schema
type Constraints struct {
	// +demi:range min=2 max=7
	Number Small `json:"number"`
	// +demi:enum fast other
	Choice Mode `json:"choice"`
	// +demi:pattern ^a[a-z]*$
	// +demi:length chars min=2 max=3
	Text Label `json:"text"`
}

// +demi:length chars min=1 max=4
// +demi:pattern ^[a-z]+$
type Label string

// +demi:root
// +demi:schema
type Numeric struct {
	Rune rune    `json:"rune"`
	Byte byte    `json:"byte"`
	I8   int8    `json:"i8"`
	I16  int16   `json:"i16"`
	I32  int32   `json:"i32"`
	I64  int64   `json:"i64"`
	Int  int     `json:"int"`
	U8   uint8   `json:"u8"`
	U16  uint16  `json:"u16"`
	U32  uint32  `json:"u32"`
	U64  uint64  `json:"u64"`
	Uint uint    `json:"uint"`
	F32  float32 `json:"f32"`
	F64  float64 `json:"f64"`
}

// A recursive result with a documented child.
//
// Paragraphs and line breaks remain product text.
// +demi:root
// +demi:schema
type Recursive struct {
	// The next child, when present.
	Next *Recursive `json:"next,omitempty"`
}

// +demi:root
// +demi:schema
type RecursiveResult struct {
	Tree Recursive `json:"tree"`
}
