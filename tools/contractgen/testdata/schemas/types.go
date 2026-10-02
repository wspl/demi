// Package schemas mirrors real command contracts for JSON Schema parity tests.
package schemas

//go:generate go run ../..

// +demi:schema
type ReadArgs struct {
	Path string `json:"path"`
}

// +demi:schema
type CreateArgs struct {
	Path    string `json:"path"`
	Content string `json:"content"`
}

// +demi:schema
type EditArgs struct {
	Path string `json:"path"`
	// +demi:length chars min=1
	Old string `json:"old"`
	New string `json:"new"`
	// +demi:range min=1
	Occurrence *uint64 `json:"occurrence,omitempty"`
	// +demi:range min=1
	Context *uint64 `json:"context,omitempty"`
}

// +demi:schema
type PatchArgs struct {
	Patch string `json:"patch"`
}

// +demi:enum commit domcontentloaded load
type Load string

// +demi:id pattern=^t[1-9][0-9]{0,14}$
type TabID string

// +demi:schema
type OpenInput struct {
	// +demi:length chars min=1 max=4096
	URL  string `json:"url"`
	Load *Load  `json:"load,omitempty"`
	// +demi:range min=1 max=300000
	Timeout *uint64 `json:"timeout,omitempty"`
}

// +demi:schema
type NavigationResult struct {
	Tab   TabID   `json:"tab"`
	URL   string  `json:"url"`
	Title *string `json:"title,omitempty"`
}

// +demi:schema
type CloseResult struct {
	Closed TabID `json:"closed"`
}

// ExampleArgs ports command-declarations' schema-settings regression.
// +demi:schema
type ExampleArgs struct {
	Path string `json:"path"`
	// +demi:range min=1 max=9
	Count   *uint32   `json:"count,omitempty"`
	Mode    *Mode     `json:"mode,omitempty"`
	Tags    []string  `json:"tags"`
	Labels  *[]string `json:"labels,omitempty"`
	NoCache *bool     `json:"no-cache,omitempty"`
}

// +demi:enum fast slow
type Mode string

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

// +demi:schema
// +demi:tolerant
type Collection struct {
	// +demi:length min=1 max=2
	Values []Small            `json:"values"`
	Labels map[string]*string `json:"labels"`
}

// +demi:range min=1 max=8
type Small uint8

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
