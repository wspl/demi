package commandwire

import (
	"bytes"
	"unicode/utf8"
)

// Edit recording limits bound the copies and journal of a job.
const (
	EditFileBytes   = 8 * 1024 * 1024
	EditJobBytes    = 64 * 1024 * 1024
	EditJobFiles    = 500
	EditJobSegments = 1000
)

// EditContext names the absolute paths of the edit directory and writer lock.
// +demi:check validateEditContext
type EditContext struct {
	Directory string `json:"directory"`
	Lock      string `json:"lock"`
}

// EditCopies names the before and after copies of one edit segment.
// +demi:check validateEditCopies
type EditCopies struct {
	// +demi:length chars min=1
	Original *string `json:"original,omitempty"`
	// +demi:length chars min=1
	Modified *string `json:"modified,omitempty"`
}

// EditKind records whether the edited file previously existed.
// +demi:enum added modified
type EditKind string

const (
	// EditAdded marks a new file.
	EditAdded EditKind = "added"
	// EditModified marks a previously existing file.
	EditModified EditKind = "modified"
)

// EditFile holds one file's edit segments.
// +demi:check validateEditFile
type EditFile struct {
	// +demi:length chars min=1
	Path string   `json:"path"`
	Kind EditKind `json:"kind"`
	// +demi:length max=1000
	Edits []EditCopies `json:"edits"`
}

// EditJournal is the bounded edit record left by a job.
type EditJournal struct {
	// +demi:length max=500
	Files []EditFile `json:"files"`
	// +demi:range max=67108864
	BytesCopied uint64 `json:"bytesCopied"`
	// +demi:range max=1000
	NextSegment    uint64 `json:"nextSegment"`
	FilesTruncated bool   `json:"filesTruncated"`
}

// IsText reports whether edit contents are UTF-8 without NUL bytes.
func IsText(data []byte) bool { return utf8.Valid(data) && !bytes.ContainsRune(data, 0) }
