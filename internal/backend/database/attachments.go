package database

import (
	"context"
	"database/sql"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// CreateAttachment records an upload of `owner`'s whose bytes `sha256` names in their
// blobs, with a text file's `snippet`.
func (c *ControlService) CreateAttachment(
	ctx context.Context,
	owner webapiproto.UserID,
	mediaType string,
	sizeBytes uint64,
	sha256 types.BlobRef,
	snippet *string,
) (AttachmentRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) (AttachmentRecord, error) {
		at, err := now.Millisecond()
		if err != nil {
			return AttachmentRecord{}, err
		}
		r := AttachmentRecord{
			ID:        webapiproto.AttachmentID(uuid.NewString()),
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
func (c *ControlService) Attachment(ctx context.Context, id webapiproto.AttachmentID) (AttachmentRecord, bool, error) {
	var found bool
	record, err := controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (AttachmentRecord, error) {
			r, ok, err := attachmentByID(ctx, tx, id)
			found = ok
			return r, err
		},
	)
	return record, found && err == nil, err
}

// UploadBlobs returns the blob of each of `owner`'s uploads, which stay for as long as the
// account (`storage.md` § Retention).
func (c *ControlService) UploadBlobs(ctx context.Context, owner webapiproto.UserID) ([]types.BlobRef, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]types.BlobRef, error) {
		return queryRecords(
			ctx,
			tx,
			"attachments",
			"SELECT DISTINCT sha256 FROM attachments WHERE user_id = ?",
			func(r *storedRow) types.BlobRef { return checked(r, "sha256", types.ParseBlobRef) },
			owner,
		)
	})
}

func attachmentRow(r *storedRow) AttachmentRecord {
	return AttachmentRecord{
		ID:        checked(r, "id", webapiproto.ParseAttachmentID),
		Owner:     checked(r, "user_id", webapiproto.ParseUserID),
		MediaType: r.text("media_type"),
		SizeBytes: r.count("size_bytes"),
		SHA256:    checked(r, "sha256", types.ParseBlobRef),
		Snippet:   r.optionalText("snippet"),
		CreatedAt: r.instant("created_at"),
	}
}

func attachmentByID(ctx context.Context, tx *sql.Tx, id webapiproto.AttachmentID) (AttachmentRecord, bool, error) {
	return queryRecord(ctx, tx, "attachments", "SELECT * FROM attachments WHERE id = ?", attachmentRow, id)
}
