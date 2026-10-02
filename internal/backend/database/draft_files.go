package database

import (
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

// DraftRefusal is why a draft was left as it was, returned through errors.As.
type DraftRefusal struct {
	Reason DraftRefusalReason
	Upload webapi.AttachmentID
}

// DraftRefusalReason identifies a draft refusal.
type DraftRefusalReason uint8

const (
	// DraftArchived means the conversation is archived.
	DraftArchived DraftRefusalReason = iota
	// DraftUploadNotFound means the caller has no upload of the named ID.
	DraftUploadNotFound
	// DraftTooLarge means the stored draft would exceed its limit.
	DraftTooLarge
	// DraftChanged means the replaced version is not the requested revision.
	DraftChanged
)

// Error describes why the draft was not changed.
func (e *DraftRefusal) Error() string {
	switch e.Reason {
	case DraftArchived:
		return "conversation is archived"
	case DraftUploadNotFound:
		return "draft upload not found: " + string(e.Upload)
	case DraftTooLarge:
		return "draft is too large"
	case DraftChanged:
		return "replaced draft has changed"
	}
	return "draft refused"
}
