package database

import (
	"context"
	"database/sql"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// CreateAttachment records an upload of `owner`'s whose bytes `sha256` names in their
// blobs, with a text file's `snippet`.
func (c *ControlService) CreateAttachment(
	ctx context.Context,
	owner webapi.UserID,
	mediaType string,
	sizeBytes uint64,
	sha256 core.BlobRef,
	snippet *string,
) (AttachmentRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) (AttachmentRecord, error) {
		at, err := now.Millisecond()
		if err != nil {
			return AttachmentRecord{}, err
		}
		r := AttachmentRecord{
			ID:        webapi.AttachmentID(uuid.NewString()),
			Owner:     owner,
			MediaType: mediaType,
			SizeBytes: sizeBytes,
			SHA256:    sha256,
			Snippet:   snippet,
			CreatedAt: now,
		}
		return r, execSQL(
			ctx,
			tx,
			"INSERT INTO attachments (id,user_id,media_type,size_bytes,sha256,snippet,created_at) VALUES (?,?,?,?,?,?,?)",
			r.ID,
			owner,
			mediaType,
			sizeBytes,
			sha256,
			snippet,
			at,
		)
	})
}

// Attachment returns the upload `id` names, whoever's it is.
func (c *ControlService) Attachment(ctx context.Context, id webapi.AttachmentID) (*AttachmentRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (*AttachmentRecord, error) {
		return attachmentByID(ctx, tx, id)
	})
}

// UploadBlobs returns the blob of each of `owner`'s uploads, which stay for as long as the
// account (`storage.md` § Retention).
func (c *ControlService) UploadBlobs(ctx context.Context, owner webapi.UserID) ([]core.BlobRef, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) ([]core.BlobRef, error) {
		return queryRecords(
			ctx,
			tx,
			"attachments",
			"SELECT DISTINCT sha256 FROM attachments WHERE user_id = ?",
			func(r *storedRow) core.BlobRef { return checked(r, "sha256", core.ParseBlobRef) },
			owner,
		)
	})
}

func attachmentRow(r *storedRow) AttachmentRecord {
	return AttachmentRecord{
		ID:        checked(r, "id", webapi.ParseAttachmentID),
		Owner:     checked(r, "user_id", webapi.ParseUserID),
		MediaType: r.text("media_type"),
		SizeBytes: r.count("size_bytes"),
		SHA256:    checked(r, "sha256", core.ParseBlobRef),
		Snippet:   r.optionalText("snippet"),
		CreatedAt: r.instant("created_at"),
	}
}

func attachmentByID(ctx context.Context, tx *sql.Tx, id webapi.AttachmentID) (*AttachmentRecord, error) {
	return queryRecord(ctx, tx, "attachments", "SELECT * FROM attachments WHERE id = ?", attachmentRow, id)
}
