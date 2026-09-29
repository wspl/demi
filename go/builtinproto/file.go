package builtinproto

// The arguments of the file.* operations. Paths are relative to the
// invocation's working directory; the handler resolves and checks them.

// ReadArgs is the input of file.read, which writes the file's bytes to stdout.
//
//demi:wire
//demi:schema
//demi:describe `file.read`: writes the file's bytes to stdout.
type ReadArgs struct {
	// File path to read
	Path string `json:"path"`
}

// CreateArgs is the input of file.create, which creates a new file and leaves
// an existing file as it is.
//
//demi:wire
//demi:schema
//demi:describe `file.create`: creates a new file; an existing file is left as it is.
type CreateArgs struct {
	// Target file path
	Path string `json:"path"`
	// File content
	Content string `json:"content"`
}

// EditArgs is the input of file.edit, which replaces one occurrence of exact
// text in an existing file.
//
//demi:wire
//demi:schema
//demi:describe `file.edit`: replaces one occurrence of exact text in an existing file.
type EditArgs struct {
	// Target file path
	Path string `json:"path"`
	// Exact text to replace
	Old string `json:"old" check:"chars=1.."`
	// Replacement text
	New string `json:"new"`
	// 1-based occurrence to replace
	Occurrence *uint `json:"occurrence,omitzero" check:"range=1.."`
	// Line number used to choose the nearest occurrence
	Context *uint `json:"context,omitzero" check:"range=1.."`
}

// PatchArgs is the input of file.patch, which applies a unified diff to one or
// more files.
//
//demi:wire
//demi:schema
//demi:describe `file.patch`: applies a unified diff to one or more files.
type PatchArgs struct {
	// Unified diff content
	Patch string `json:"patch"`
}
