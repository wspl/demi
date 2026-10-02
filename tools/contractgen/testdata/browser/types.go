// Package browser exercises the remaining browser wire shapes.
package browser

//go:generate go run ../.. .

// A node's value: an input's text, or a range or progress number.
//
// +demi:root direction=receive output=plugin-browser
// +demi:schema
// +demi:msgpack
// +demi:union untagged
//
//sumtype:decl
type NodeValue interface{ nodeValue() }

// +demi:variant
type NodeValueText string

func (*NodeValueText) nodeValue() {}

// +demi:variant
type NodeValueNumber float64

func (*NodeValueNumber) nodeValue() {}

// +demi:root direction=receive output=plugin-browser
// +demi:msgpack
// +demi:union untagged
//
//sumtype:decl
type Scalar interface{ scalar() }

// +demi:variant
// +demi:range min=-10 max=-1
type First int32

func (*First) scalar() {}

// +demi:variant
type Second float64

func (*Second) scalar() {}

// +demi:variant
type Boolean bool

func (*Boolean) scalar() {}

// +demi:variant
type Object struct {
	Value string `json:"value"`
}

func (*Object) scalar() {}

// +demi:root direction=receive output=plugin-browser
// +demi:msgpack
type Wheel struct {
	// +demi:range min=-10000.0 max=10000.0
	DeltaX float64 `json:"deltaX"`
	// +demi:range min=-10000.0 max=10000.0
	DeltaY float64 `json:"deltaY"`
}

// What a failure knows beyond its code and message.
//
// +demi:root
// +demi:schema
// +demi:msgpack
// +demi:tolerant
type ErrorDetails struct {
	Action *ActionProgress `json:"action,omitempty"`
	Tab    *string         `json:"tab,omitempty"`
	// The tab's URL when the action failed.
	URL *string `json:"url,omitempty"`
	// The element condition that was not met.
	Condition *string `json:"condition,omitempty"`
	// What intercepted the pointer instead of the target.
	Interceptor *string `json:"interceptor,omitempty"`
	// How many elements an ambiguous target matched.
	Count *uint `json:"count,omitempty"`
	// How many characters `type` delivered before it failed.
	Delivered *uint `json:"delivered,omitempty"`
	// The numbers of the agents whose debugging connections hold the tab
	// when a command on it times out.
	DebuggingCallers *[]uint64 `json:"debuggingCallers,omitempty"`
	// What an export wrote before it failed or was interrupted.
	*AssetsExportResult
}

// How far an action's input got when it failed: not delivered, delivered
// with a later wait failing, or lost with the connection after delivery
// began.
//
// +demi:enum not_started completed unknown
type ActionProgress string

type AssetsExportResult struct {
	Directory string          `json:"directory"`
	Manifest  string          `json:"manifest"`
	Files     []ExportedAsset `json:"files"`
}

// An asset `assets.export` saved.
type ExportedAsset struct {
	ID       string `json:"id"`
	Path     string `json:"path"`
	Bytes    uint   `json:"bytes"`
	MIMEType string `json:"mimeType"`
}

// What `dialog.inspect` answers: the dialog, or `null` when none is open.
//
// +demi:root direction=receive output=plugin-browser
// +demi:schema
type DialogInspectResult struct {
	// +demi:nullable
	Dialog *Dialog `json:"dialog"`
}

// A JavaScript dialog a tab shows.
type Dialog struct {
	Type    DialogType `json:"type"`
	Message string     `json:"message"`
}

// +demi:enum alert confirm prompt beforeunload
type DialogType string

// +demi:root direction=receive output=plugin-browser
// +demi:msgpack
type Optional struct {
	Before string `json:"before"`
	*Dialog
	After string `json:"after"`
}

// A failed operation, or a failed item of a batch such as a page of
// `content.fetch`.
//
// +demi:root
// +demi:schema
type BrowserFailure struct {
	Code    BrowserErrorCode `json:"code"`
	Message string           `json:"message"`
	Details *ErrorDetails    `json:"details,omitempty"`
}

// Why an operation failed. Each cause keeps its own code.
//
// +demi:enum invalid_input tab_not_found tab_busy stale_ref stale_cursor stale_inventory stale_tools target_not_found ambiguous_target not_actionable timeout dialog_blocked dialog_not_found invalid_dialog_action history_boundary navigation_failed protected_value side_effect_rejected unsupported_result unsupported_capability cdp_method_denied output_exists io_error result_too_large partial_failure driver_error browser_unavailable browser_lost cancelled outcome_unknown
type BrowserErrorCode string
