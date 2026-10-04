package store

import (
	"bytes"
	"context"
	"sync/atomic"

	"golang.org/x/sync/errgroup"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// Blobs is the conversation owner's blob namespace as reached by a session.
// Implementations support concurrent calls.
type Blobs interface {
	// Put stores bytes under their SHA-256 unless already present, returning that name.
	Put(ctx context.Context, data core.B64Bytes) (core.BlobRef, error)
	// Read returns a blob's bytes and whether it exists; missing is not an error.
	Read(ctx context.Context, blob core.BlobRef) (core.B64Bytes, bool, error)
}

// heldMedium is what a session holds for one medium: its bytes, or the knowledge that the namespace lacks it.
//
//sumtype:decl
type heldMedium interface{ heldMedium() }

// heldBytes carries a medium's bytes.
type heldBytes struct {
	// Bytes holds the medium contents.
	bytes core.B64Bytes
}

// heldMissing records that the namespace does not hold the medium's blob.
type heldMissing struct{}

func (*heldBytes) heldMedium()   {}
func (*heldMissing) heldMedium() {}

// HeldMedia holds bytes or known absence by blob name. Its zero value is empty.
// Its session serializes mutation; selections and views own their snapshots.
type HeldMedia struct{ media map[core.BlobRef]heldMedium }

// Hold keeps bytes unless something is already held for the blob.
func (h *HeldMedia) Hold(blob core.BlobRef, data core.B64Bytes) {
	if h.media == nil {
		h.media = map[core.BlobRef]heldMedium{}
	}
	if _, exists := h.media[blob]; !exists {
		h.media[blob] = &heldBytes{bytes: bytes.Clone(data)}
	}
}

// Absorb holds what other holds, keeping what is held already.
func (h *HeldMedia) Absorb(other HeldMedia) {
	if h.media == nil {
		h.media = map[core.BlobRef]heldMedium{}
	}
	for blob, held := range other.media {
		if _, exists := h.media[blob]; !exists {
			h.media[blob] = copyHeld(held)
		}
	}
}

// Retain releases media not named in referenced.
func (h *HeldMedia) Retain(referenced map[core.BlobRef]struct{}) {
	for blob := range h.media {
		if _, keep := referenced[blob]; !keep {
			delete(h.media, blob)
		}
	}
}

// Select copies the held media that blocks reference.
func (h *HeldMedia) Select(blocks []core.Block) HeldMedia {
	selected := HeldMedia{media: map[core.BlobRef]heldMedium{}}
	for _, block := range blocks {
		for _, blob := range References(block) {
			if held, exists := h.media[blob]; exists {
				selected.media[blob] = copyHeld(held)
			}
		}
	}
	return selected
}

// ModelView is replayed transcript blocks and the media held for their requests.
// Treat its blocks and the bytes returned by Held as immutable.
type ModelView struct {
	// Start is where the blocks start in the transcript.
	Start int
	// Blocks contains immutable transcript snapshots in replay order.
	Blocks []core.Block
	media  HeldMedia
}

// NewModelView returns a view or the distinct blobs that must first be read.
// A nonempty missing list means the returned view is nil.
func NewModelView(start int, blocks []core.Block, held HeldMedia) (*ModelView, []core.BlobRef) {
	missing := []core.BlobRef{}
	seen := map[core.BlobRef]bool{}
	for _, block := range blocks {
		for _, blob := range References(block) {
			if _, exists := held.media[blob]; !exists && !seen[blob] {
				missing = append(missing, blob)
				seen[blob] = true
			}
		}
	}
	if len(missing) != 0 {
		return nil, missing
	}
	return &ModelView{Start: start, Blocks: cloneModelBlocks(blocks), media: held.Select(blocks)}, nil
}

// Held returns a copy of the bytes held for a referenced blob, or false when
// the namespace does not hold that blob. An unreferenced blob violates the
// view invariant and panics.
func (v *ModelView) Held(blob core.BlobRef) (core.B64Bytes, bool) {
	held, exists := v.media.media[blob]
	if !exists {
		panic("the model's view holds something for every medium its blocks reference")
	}
	stored, found := held.(*heldBytes)
	if !found {
		return nil, false
	}
	return bytes.Clone(stored.bytes), true
}

// MissingText renders a missing image, video or document for a model request.
func MissingText(kind string) string { return "[missing " + kind + "]" }

// References lists the blobs a block's media reference, in part order.
func References(block core.Block) []core.BlobRef {
	switch block := block.(type) {
	case *core.UserBlock:
		return ContentReferences(block.Content)
	case *core.SteerBlock:
		return ContentReferences(block.Content)
	case *core.ToolCallBlock:
		refs := []core.BlobRef{}
		for _, part := range block.Output {
			var source core.ToolMediaSource
			switch part := part.(type) {
			case *core.ToolImage:
				source = part.Source
			case *core.ToolVideo:
				source = part.Source
			case *core.ToolText, *core.ToolGone:
				continue
			}
			if ref, ok := source.(*core.ToolMediaRef); ok {
				refs = append(refs, ref.Ref)
			}
		}
		return refs
	case *core.ContextBlock,
		*core.WakeupBlock,
		*core.AgentMessageBlock,
		*core.ResumeBlock,
		*core.AbortBlock,
		*core.ThinkingBlock,
		*core.RedactedThinkingBlock,
		*core.TextBlock,
		*core.ResponseBlock,
		*core.ErrorBlock,
		*core.CompactionBoundaryBlock,
		*core.CompactionMarkerBlock:
		return nil
	}
	return nil
}

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
	// Blob identifies the referenced bytes.
	Blob core.BlobRef
	// Holder identifies the retention category.
	Holder Holder
}

// BlockReferences lists media in part order, then edit copies in file and
// segment order, original before modified.
func BlockReferences(block core.Block) []BlockReference {
	holder := Message
	call, tool := block.(*core.ToolCallBlock)
	if tool {
		holder = ToolResult
	}
	refs := []BlockReference{}
	for _, blob := range References(block) {
		refs = append(refs, BlockReference{Blob: blob, Holder: holder})
	}
	if tool {
		if view, ok := call.View.(*core.ShellView); ok && view.Files != nil {
			for _, file := range *view.Files {
				for _, edit := range file.Edits {
					if edit.Copies != nil {
						refs = append(
							refs,
							BlockReference{Blob: edit.Copies.Original, Holder: EditCopy},
							BlockReference{Blob: edit.Copies.Modified, Holder: EditCopy},
						)
					}
				}
			}
		}
	}
	return refs
}

// ContentReferences lists media references in a message's or steer's content.
func ContentReferences(content []core.UserContentBlock) []core.BlobRef {
	refs := []core.BlobRef{}
	for _, part := range content {
		var source core.MediaSource
		switch part := part.(type) {
		case *core.UserImage:
			source = part.Source
		case *core.UserVideo:
			source = part.Source
		case *core.UserDocument:
			if ref, ok := part.Source.(*core.DocumentRef); ok {
				refs = append(refs, ref.Ref)
			}
			continue
		case *core.UserText, *core.UserReference, *core.UserAttachment:
			continue
		}
		if ref, ok := source.(*core.MediaSourceRef); ok {
			refs = append(refs, ref.Ref)
		}
	}
	return refs
}

// PersistResult stores tool media once as it enters the transcript and holds its
// bytes. A failed put becomes a gone part containing the store's error.
func PersistResult(
	ctx context.Context,
	output []provider.ResultPart,
	blobs Blobs,
) ([]core.ToolResultContentBlock, HeldMedia) {
	held := HeldMedia{}
	stored := make([]core.ToolResultContentBlock, 0, len(output))
	for _, part := range output {
		var data provider.MediaBytes
		var kind core.ModelMediaKind
		switch part := part.(type) {
		case *provider.TextPart:
			stored = append(stored, &core.ToolText{Text: part.Text})
			continue
		case *provider.ResultImage:
			data = part.Bytes
			kind = "image"
		case *provider.ResultVideo:
			data = part.Bytes
			kind = "video"
		}
		blob, err := blobs.Put(ctx, data.Data)
		if err != nil {
			stored = append(
				stored,
				&core.ToolGone{Kind: kind, MediaType: data.MediaType, Cause: &core.NotStored{Error: err.Error()}},
			)
			continue
		}
		held.Hold(blob, data.Data)
		source := &core.ToolMediaRef{Ref: blob, MediaType: data.MediaType}
		if kind == "image" {
			stored = append(stored, &core.ToolImage{Source: source})
		} else {
			stored = append(stored, &core.ToolVideo{Source: source})
		}
	}
	return stored, held
}

// ReadMedia reads blobs at most eight at a time, holding bytes or known absence.
func ReadMedia(ctx context.Context, blobs Blobs, refs []core.BlobRef) (HeldMedia, error) {
	found := make([]heldMedium, len(refs))
	var next atomic.Uint64
	group, readCtx := errgroup.WithContext(ctx)
	for range min(8, len(refs)) {
		group.Go(func() error {
			for {
				if err := readCtx.Err(); err != nil {
					return err
				}
				index := next.Add(1) - 1
				if index >= uint64(len(refs)) {
					return nil
				}
				data, exists, err := blobs.Read(readCtx, refs[index])
				if err != nil {
					return err
				}
				if exists {
					found[index] = &heldBytes{bytes: bytes.Clone(data)}
				} else {
					found[index] = &heldMissing{}
				}
			}
		})
	}
	if err := group.Wait(); err != nil {
		return HeldMedia{}, err
	}
	held := HeldMedia{media: map[core.BlobRef]heldMedium{}}
	for index, blob := range refs {
		held.media[blob] = found[index]
	}
	return held, nil
}

// copyHeld preserves a session's ownership of a medium's bytes.
func copyHeld(held heldMedium) heldMedium {
	switch held := held.(type) {
	case *heldBytes:
		return &heldBytes{bytes: bytes.Clone(held.bytes)}
	case *heldMissing:
		return &heldMissing{}
	}
	return nil
}
