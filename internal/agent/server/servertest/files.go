package servertest

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// ClientText is a message of one text, as the web app sends it.
func ClientText(text string) []framewire.ClientContent { panic("not written: a-server") }

// TestFiles holds uploads and the blocks and media bytes they resolve to.
// Every other reference is refused. Its zero value is ready for use and
// methods support concurrent calls.
type TestFiles struct{}

// NewFiles constructs an empty upload resolver.
func NewFiles() *TestFiles { panic("not written: a-server") }

// Upload records the blocks and held media an upload reference resolves to.
func (f *TestFiles) Upload(reference string, blocks []core.UserContentBlock, media store.HeldMedia) {
	panic("not written: a-server")
}

// Resolve returns uploads in order or refuses the entire frame.
func (f *TestFiles) Resolve(ctx context.Context, files []server.FileReference) (server.ResolvedFiles, error) {
	panic("not written: a-server")
}

var _ server.ContentResolver = (*TestFiles)(nil)
