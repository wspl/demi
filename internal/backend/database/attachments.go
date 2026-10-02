package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// CreateAttachment records an upload of `owner`'s whose bytes `sha256` names in their
// blobs, with a text file's `snippet`.
func (c *ControlService) CreateAttachment(ctx context.Context, owner webapi.UserID, mediaType string, sizeBytes uint64, sha256 core.BlobRef, snippet *string) (AttachmentRecord, error) {
	panic("not written: b-database")
}

// Attachment returns the upload `id` names, whoever's it is.
func (c *ControlService) Attachment(ctx context.Context, id webapi.AttachmentID) (*AttachmentRecord, error) {
	panic("not written: b-database")
}

// UploadBlobs returns the blob of each of `owner`'s uploads, which stay for as long as the
// account (`storage.md` § Retention).
func (c *ControlService) UploadBlobs(ctx context.Context, owner webapi.UserID) ([]core.BlobRef, error) {
	panic("not written: b-database")
}
