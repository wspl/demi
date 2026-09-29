package webapi

import (
	"github.com/wspl/demi/go/core"
)

// `POST /attachments?name=`: the file's name, which decides whether the
// answer carries a text file's opening, as a message with the file does.
//
//demi:wire
type UploadQuery struct {
	Name string `json:"name" check:"chars=1..255"`
}

// The answer of an upload.
//
//demi:wire open
type AttachmentAnswer struct {
	Attachment AttachmentDTO `json:"attachment"`
}

// An upload as the composer shows it: the media type the backend read from
// the file's bytes when it recognizes them, else the one it was sent with,
// and a text file's opening.
//
//demi:wire open
type AttachmentDTO struct {
	ID        AttachmentID `json:"id" check:"func=Validate"`
	MediaType string       `json:"mediaType"`
	SizeBytes uint64       `json:"sizeBytes" check:"range=..9007199254740991"`
	// The bytes in the caller's blobs.
	SHA256    core.BlobRef   `json:"sha256" check:"func=core.Validate"`
	CreatedAt core.Timestamp `json:"createdAt" check:"func=core.Validate"`
	Snippet   *string        `json:"snippet,omitzero"`
}
