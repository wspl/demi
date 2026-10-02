package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// TextRefusal says why a file is not shown as text. Use errors.Is to compare it.
type TextRefusal string

const (
	// TextTooLarge means the file exceeds the edit snapshot byte limit.
	TextTooLarge TextRefusal = "The file is too large to show"
	// TextNotText means the file is not UTF-8 without NUL bytes.
	TextNotText TextRefusal = "The file is not UTF-8 text"
)

// Error returns the refusal shown to the user.
func (r TextRefusal) Error() string { panic("not written: b-runners") }

// TextOf interprets bytes as UTF-8 without NUL bytes, within the edit snapshot limit.
func TextOf(bytes []byte) (string, error) { panic("not written: b-runners") }

// ReadTextFile reads one Host file as text; an oversized file is refused before
// reading its bytes. Errors retain the host.Error or TextRefusal cause.
func ReadTextFile(ctx context.Context, fs host.FS, path string) (string, error) {
	panic("not written: b-runners")
}

// BrowseDirectory reads entries and metadata sequentially so one listing cannot
// flood the runner queue. Entries that disappear meanwhile are omitted.
func BrowseDirectory(ctx context.Context, fs host.FS, path string) ([]webapi.DirectoryEntry, error) {
	panic("not written: b-runners")
}
