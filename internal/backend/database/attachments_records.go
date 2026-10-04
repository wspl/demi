package database

import (
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// AttachmentRecord is an upload as its record holds it.
type AttachmentRecord struct {
	ID        webapiproto.AttachmentID
	Owner     webapiproto.UserID
	MediaType string
	SizeBytes uint64
	SHA256    types.BlobRef
	// A text file's opening, as the upload's answer carried it.
	Snippet   *string
	CreatedAt types.Timestamp
}
