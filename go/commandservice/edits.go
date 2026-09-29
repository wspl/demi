package commandservice

import (
	"errors"
	"path/filepath"

	"github.com/google/jsonschema-go/jsonschema"
)

// The limits of a job's edit record that its journal states.
const (
	// EditJobBytes is the most bytes the record copies in total.
	EditJobBytes = 64 * 1024 * 1024
	// EditJobFiles is the most files the record lists.
	EditJobFiles = 500
	// EditJobSegments is the most edit segments the record holds.
	EditJobSegments = 1000
)

// An EditContext says where an invoked command records its edits: the job's
// edit directory and the lock that serializes writers to it. Both paths are
// absolute.
type EditContext struct {
	Directory string `json:"directory"`
	Lock      string `json:"lock"`
}

var editContextWire = declare(func(s *jsonschema.Schema) {
	prop(s, "directory").Pattern = patternNonEmptyNoNUL
	prop(s, "lock").Pattern = patternNonEmptyNoNUL
}, checkEditContext)

func checkEditContext(v *EditContext) error {
	if !filepath.IsAbs(v.Directory) {
		return errors.New("directory: is not an absolute path")
	}
	if !filepath.IsAbs(v.Lock) {
		return errors.New("lock: is not an absolute path")
	}
	return nil
}

// EditCopies holds the copies of one edit segment: the file before and after
// it.
type EditCopies struct {
	Original *string `json:"original,omitzero"`
	Modified *string `json:"modified,omitzero"`
}

var editCopiesWire = declare[EditCopies](func(s *jsonschema.Schema) {
	prop(s, "original").Pattern = patternNonEmptyNoNUL
	prop(s, "modified").Pattern = patternNonEmptyNoNUL
}, nil)

// An EditKind says whether an edited file existed before the job.
type EditKind string

// The kinds of edited file.
const (
	EditAdded    EditKind = "added"
	EditModified EditKind = "modified"
)

// An EditFile is one edited file and its segments.
type EditFile struct {
	Path  string       `json:"path"`
	Kind  EditKind     `json:"kind"`
	Edits []EditCopies `json:"edits"`
}

var editFileWire = declare[EditFile](func(s *jsonschema.Schema) {
	prop(s, "path").Pattern = patternNonEmptyNoNUL
	prop(s, "kind").Enum = []any{string(EditAdded), string(EditModified)}
	prop(s, "edits").MaxItems = jsonschema.Ptr(EditJobSegments)
}, nil, editCopiesWire)

// An EditJournal is a job's edit record as the command left it.
type EditJournal struct {
	Files          []EditFile `json:"files"`
	BytesCopied    uint64     `json:"bytesCopied"`
	NextSegment    uint64     `json:"nextSegment"`
	FilesTruncated bool       `json:"filesTruncated"`
}

var _ = declare[EditJournal](func(s *jsonschema.Schema) {
	prop(s, "files").MaxItems = jsonschema.Ptr(EditJobFiles)
	prop(s, "bytesCopied").Maximum = jsonschema.Ptr(float64(EditJobBytes))
	prop(s, "nextSegment").Maximum = jsonschema.Ptr(float64(EditJobSegments))
}, nil, editFileWire)
