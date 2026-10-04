package webapiproto

import (
	"github.com/wspl/demi/internal/types"
)

// The most bytes an upload holds.
const AttachmentMaxBytes = 25 * 1024 * 1024

// `POST /attachments?name=`: the file's name, which decides whether the
// answer carries a text file's opening, as a message with the file does.
// +demi:root
type UploadQuery struct {
	// +demi:length chars min=1 max=255
	Name string `json:"name"`
}

// The answer of an upload.
// +demi:root direction=receive output=web
// +demi:tolerant
type AttachmentAnswer struct {
	Attachment AttachmentDTO `json:"attachment"`
}

// An upload as the composer shows it: the media type the backend read from
// the file's bytes when it recognizes them, else the one it was sent with,
// and a text file's opening.
// +demi:tolerant
type AttachmentDTO struct {
	ID        AttachmentID `json:"id"`
	MediaType string       `json:"mediaType"`
	// +demi:range max=9007199254740991
	SizeBytes uint64 `json:"sizeBytes"`
	// The bytes in the caller's blobs.
	Sha256    types.BlobRef   `json:"sha256"`
	CreatedAt types.Timestamp `json:"createdAt"`
	Snippet   *string         `json:"snippet,omitempty"`
}
