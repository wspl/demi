package fileop

//go:generate go run github.com/wspl/demi/tools/contractgen

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
