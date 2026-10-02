package server

import (
	"context"
	"fmt"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// FileReference is a file a message refers to that only the backend can resolve.
// It is internal data, not a second declaration of a client wire contract.
//
//sumtype:decl
type FileReference interface{ fileReference() }

// Upload refers to an upload by attachment id, written under FileName.
type Upload struct {
	Ref      string
	FileName string
}

// RemoteFile refers to a file on a paired device.
type RemoteFile struct {
	DeviceID string
	Path     string
}

func (*Upload) fileReference()     {}
func (*RemoteFile) fileReference() {}

// ContentResolver resolves the files of one frame's content in the backend.
// An upload becomes its native media block, when present, and attachment
// record, or unavailable text; a remote file becomes its allowed reference.
type ContentResolver interface {
	// Resolve resolves all references together in the given order: none is
	// granted unless all can be. A failure refuses the frame with *ContentError.
	Resolve(ctx context.Context, files []FileReference) (ResolvedFiles, error)
}

// ResolvedFiles holds each reference's blocks in order, with media by reference
// and the bytes the backend read, which the session holds.
type ResolvedFiles struct {
	Blocks [][]core.UserContentBlock
	Media  store.HeldMedia
}

// ContentError explains why a frame's files could not be resolved.
type ContentError struct {
	Message string
	// Code is the backend error-frame code, such as frame_delivery_failed.
	Code  *string
	Cause error
}

// Error returns the refusal's text.
func (e *ContentError) Error() string { return e.Message }

// Unwrap returns the underlying resolution failure.
func (e *ContentError) Unwrap() error { return e.Cause }

// resolveEdit replaces a frame's external file references while retaining its order.
func resolveEdit(ctx context.Context, resolver ContentResolver, content []framewire.ClientContent) ([]session.EditContent, store.HeldMedia, error) {
	files := []FileReference{}
	for _, item := range content {
		switch v := item.(type) {
		case *framewire.UploadContent:
			files = append(files, &Upload{Ref: v.Ref, FileName: v.FileName})
		case *framewire.RemoteFileContent:
			files = append(files, &RemoteFile{DeviceID: v.DeviceID, Path: v.Path})
		case *framewire.TextContent, *framewire.ReferenceContent, *framewire.MediaContent, *framewire.AttachmentContent:
		}
	}
	var resolved ResolvedFiles
	if len(files) > 0 {
		var err error
		resolved, err = resolver.Resolve(ctx, files)
		if err != nil {
			return nil, store.HeldMedia{}, err
		}
	}
	if len(resolved.Blocks) != len(files) {
		return nil, store.HeldMedia{}, &ContentError{Message: fmt.Sprintf("the backend resolved %d of %d file references", len(resolved.Blocks), len(files))}
	}
	parts := []session.EditContent{}
	next := 0
	for _, item := range content {
		switch v := item.(type) {
		case *framewire.TextContent:
			parts = append(parts, &session.Content{Block: &core.UserText{Text: v.Text}})
		case *framewire.ReferenceContent:
			parts = append(parts, &session.Content{Block: &core.UserReference{Reference: v.Reference}})
		case *framewire.UploadContent, *framewire.RemoteFileContent:
			for _, block := range resolved.Blocks[next] {
				parts = append(parts, &session.Content{Block: block})
			}
			next++
		case *framewire.MediaContent:
			parts = append(parts, &session.KeptMedia{Media: v.Media})
		case *framewire.AttachmentContent:
			parts = append(parts, &session.KeptAttachment{Path: v.Path})
		}
	}
	return parts, resolved.Media, nil
}

// resolveMessage gives a validated send or steer its resolved content and media.
func resolveMessage(ctx context.Context, resolver ContentResolver, content []framewire.ClientContent) ([]core.UserContentBlock, store.HeldMedia, error) {
	parts, media, err := resolveEdit(ctx, resolver, content)
	if err != nil {
		return nil, media, err
	}
	blocks := make([]core.UserContentBlock, 0, len(parts))
	for _, part := range parts {
		switch v := part.(type) {
		case *session.Content:
			blocks = append(blocks, v.Block)
		case *session.KeptMedia, *session.KeptAttachment:
			return nil, media, &ContentError{Message: "Only an edit refers to the files its message holds"}
		}
	}
	return blocks, media, nil
}
