package server

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
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
func (e *ContentError) Error() string { panic("not written: a-server") }

// Unwrap returns the underlying resolution failure.
func (e *ContentError) Unwrap() error { panic("not written: a-server") }
