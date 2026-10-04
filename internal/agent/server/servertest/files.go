package servertest

import (
	"context"
	"fmt"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
)

// ClientText is a message of one text, as the web app sends it.
func ClientText(text string) []conversationproto.ClientContent {
	return []conversationproto.ClientContent{&conversationproto.TextContent{Text: text}}
}

// TestFiles holds uploads and the blocks and media bytes they resolve to.
// Every other reference is refused. Its zero value is ready for use and
// methods support concurrent calls.
type TestFiles struct {
	mu      sync.Mutex // Protects the uploaded fixtures.
	uploads map[string]server.ResolvedFiles
}

// NewFiles constructs an empty upload resolver.
func NewFiles() *TestFiles { return &TestFiles{} }

// Upload records the blocks and held media an upload reference resolves to.
func (f *TestFiles) Upload(reference string, blocks []types.UserContentBlock, media store.HeldMedia) {
	var owned store.HeldMedia
	owned.Absorb(media)
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.uploads == nil {
		f.uploads = map[string]server.ResolvedFiles{}
	}
	f.uploads[reference] = server.ResolvedFiles{Blocks: [][]types.UserContentBlock{slices.Clone(blocks)}, Media: owned}
}

// Resolve returns uploads in order or refuses the entire frame.
func (f *TestFiles) Resolve(_ context.Context, files []server.FileReference) (server.ResolvedFiles, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	var result server.ResolvedFiles
	for _, file := range files {
		switch v := file.(type) {
		case *server.Upload:
			uploaded, ok := f.uploads[v.Ref]
			if !ok {
				return server.ResolvedFiles{}, &server.ContentError{
					Message: fmt.Sprintf("upload %s is not available", v.Ref),
					Code:    new("frame_delivery_failed"),
				}
			}
			result.Blocks = append(result.Blocks, slices.Clone(uploaded.Blocks[0]))
			result.Media.Absorb(uploaded.Media)
		case *server.RemoteFile:
			return server.ResolvedFiles{}, &server.ContentError{
				Message: fmt.Sprintf("device %s is not paired", v.DeviceID),
				Code:    new("frame_delivery_failed"),
			}
		}
	}
	return result, nil
}

var _ server.ContentResolver = (*TestFiles)(nil)
