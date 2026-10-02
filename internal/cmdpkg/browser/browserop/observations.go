package browserop

// A node's value: an input's text, or a range or progress number.
// +demi:union untagged
//
// +demi:root
//
//sumtype:decl
type NodeValue interface{ nodeValue() }

// +demi:variant
type NodeValueText string

func (*NodeValueText) nodeValue() {}

// +demi:variant
type NodeValueNumber float64

func (*NodeValueNumber) nodeValue() {}

// One accessibility node that `find` matched or `probe` found under a point.
// +demi:root
type BrowserNode struct {
	Ref    *NodeRef   `json:"ref,omitempty"`
	Role   string     `json:"role"`
	Name   string     `json:"name"`
	Value  *NodeValue `json:"value,omitempty"`
	Depth  uint       `json:"depth"`
	States []string   `json:"states"`
	Bounds *Bounds    `json:"bounds,omitempty"`
}

// One node of the tree `inspect` returns: an accessibility node, or a DOM
// element with its tag.
// +demi:root
type BrowserTreeNode struct {
	Ref      *NodeRef           `json:"ref,omitempty"`
	Role     *string            `json:"role,omitempty"`
	Name     *string            `json:"name,omitempty"`
	Value    *NodeValue         `json:"value,omitempty"`
	Tag      *string            `json:"tag,omitempty"`
	States   *[]string          `json:"states,omitempty"`
	Children *[]BrowserTreeNode `json:"children,omitempty"`
}

// +demi:schema
// +demi:root
type InspectResult struct {
	Tab       TabID             `json:"tab"`
	URL       string            `json:"url"`
	Title     string            `json:"title"`
	View      InspectView       `json:"view"`
	Tree      []BrowserTreeNode `json:"tree"`
	Truncated bool              `json:"truncated"`
}

// What `find` answers. `count` is every current match, even when `offset`
// and `limit` return fewer.
// +demi:schema
// +demi:root
type FindResult struct {
	Matches   []BrowserNode `json:"matches"`
	Count     uint          `json:"count"`
	Truncated bool          `json:"truncated"`
}

// +demi:schema
// +demi:root
type ProbeResult struct {
	Matches   []BrowserNode   `json:"matches"`
	Viewport  BrowserViewport `json:"viewport"`
	Path      *string         `json:"path,omitempty"`
	Truncated bool            `json:"truncated"`
}
