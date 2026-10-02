package database

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// AttachmentRecord is an upload as its record holds it.
type AttachmentRecord struct {
	ID        webapi.AttachmentID
	Owner     webapi.UserID
	MediaType string
	SizeBytes uint64
	SHA256    core.BlobRef
	// A text file's opening, as the upload's answer carried it.
	Snippet   *string
	CreatedAt core.Timestamp
}
