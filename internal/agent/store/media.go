package store

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// BlobStore is the conversation owner's blob namespace as reached by a session.
// Implementations support concurrent calls.
type BlobStore interface {
	// Put stores bytes under their SHA-256 unless already present, returning that name.
	Put(ctx context.Context, data core.B64Bytes) (core.BlobRef, error)
	// Read returns a blob's bytes and whether it exists; missing is not an error.
	Read(ctx context.Context, blob core.BlobRef) (core.B64Bytes, bool, error)
}

// Held is what a session holds for one medium.
//
//sumtype:decl
type Held interface{ held() }

// HeldBytes carries a medium's bytes.
type HeldBytes struct{ Bytes core.B64Bytes }

// HeldMissing records that the namespace does not hold the medium's blob.
type HeldMissing struct{}

func (*HeldBytes) held()   {}
func (*HeldMissing) held() {}

// HeldMedia holds bytes or known absence by blob name. Its zero value is empty.
// Its session serializes mutation; selections and views own their snapshots.
type HeldMedia struct{}

// Hold keeps bytes unless something is already held for the blob.
func (h *HeldMedia) Hold(blob core.BlobRef, data core.B64Bytes) { panic("not written: a-store") }

// Absorb holds what other holds, keeping what is held already.
func (h *HeldMedia) Absorb(other HeldMedia) { panic("not written: a-store") }

// Retain releases media not named in referenced.
func (h *HeldMedia) Retain(referenced map[core.BlobRef]struct{}) { panic("not written: a-store") }

// Select copies the held media that blocks reference.
func (h *HeldMedia) Select(blocks []core.Block) HeldMedia { panic("not written: a-store") }

// ModelView is replayed transcript blocks and the media held for their requests.
// Treat its blocks and the bytes returned by Held as immutable.
type ModelView struct {
	// Start is where the blocks start in the transcript.
	Start  int
	Blocks []core.Block
}

// NewModelView returns a view or the distinct blobs that must first be read.
// A nonempty missing list means the returned view is nil.
func NewModelView(start int, blocks []core.Block, held HeldMedia) (*ModelView, []core.BlobRef) {
	panic("not written: a-store")
}

// Held returns the media held for a referenced blob, or nil if it is outside the view.
func (v *ModelView) Held(blob core.BlobRef) Held { panic("not written: a-store") }

// MissingText renders a missing image, video or document for a model request.
func MissingText(kind string) string { panic("not written: a-store") }

// References lists the blobs a block's media reference, in part order.
func References(block core.Block) []core.BlobRef { panic("not written: a-store") }

// Holder identifies what in a block holds a referenced blob.
type Holder uint8

const (
	// Message holds a message's or steer's medium from user uploads.
	Message Holder = iota
	// ToolResult holds a tool's image or video, subject to retirement.
	ToolResult
	// EditCopy holds a side of a file edited by a shell command, seen only by the user.
	EditCopy
)

// BlockReference is a blob indexed by the store for retention.
type BlockReference struct {
	Blob   core.BlobRef
	Holder Holder
}

// BlockReferences lists media in part order, then edit copies in file and
// segment order, original before modified.
func BlockReferences(block core.Block) []BlockReference { panic("not written: a-store") }

// ContentReferences lists media references in a message's or steer's content.
func ContentReferences(content []core.UserContentBlock) []core.BlobRef { panic("not written: a-store") }

// PersistResult stores tool media once as it enters the transcript and holds its
// bytes. A failed put becomes a gone part containing the store's error.
func PersistResult(ctx context.Context, output []provider.ResultPart, blobs BlobStore) ([]core.ToolResultContentBlock, HeldMedia) {
	panic("not written: a-store")
}

// ReadMedia reads blobs at most eight at a time, holding bytes or known absence.
func ReadMedia(ctx context.Context, blobs BlobStore, refs []core.BlobRef) (HeldMedia, error) {
	panic("not written: a-store")
}
