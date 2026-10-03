package database

import (
	"errors"

	"github.com/wspl/demi/internal/webapi"
)

// StagedFile names an upload of the caller or a file on a paired device.
//
//sumtype:decl
type StagedFile interface{ stagedFile() }

// StagedUpload names an uploaded file.
type StagedUpload struct {
	ID       webapi.AttachmentID
	FileName string
}

func (*StagedUpload) stagedFile() {}

// StagedRemote names a file on a paired device.
type StagedRemote struct {
	DeviceID string
	Path     string
}

func (*StagedRemote) stagedFile() {}

var (
	// ErrDraftTooLarge means the stored draft would exceed its limit.
	ErrDraftTooLarge = errors.New("draft is too large")
	// ErrDraftChanged means the replaced version is not the requested revision.
	ErrDraftChanged = errors.New("replaced draft has changed")
)

// UploadNotFoundError means the caller has no upload of the named ID.
type UploadNotFoundError struct {
	// Upload is the ID the draft named.
	Upload webapi.AttachmentID
}

// Error names the missing upload.
func (e *UploadNotFoundError) Error() string { return "draft upload not found: " + string(e.Upload) }
