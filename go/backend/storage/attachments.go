package storage

import (
	"context"
	"database/sql"
	"errors"

	"github.com/google/uuid"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type AttachmentRecord struct {
	ID        webapi.AttachmentID
	Owner     webapi.UserID
	MediaType string
	SizeBytes uint64
	SHA256    core.BlobRef
	Snippet   *string
	CreatedAt core.Timestamp
}

func (c *Control) CreateAttachment(ctx context.Context, owner webapi.UserID, mediaType string, size uint64, hash core.BlobRef, snippet *string) (AttachmentRecord, error) {
	id, err := webapi.ParseAttachmentID(uuid.NewString())
	if err != nil {
		return AttachmentRecord{}, err
	}
	record := AttachmentRecord{id, owner, mediaType, size, hash, snippet, c.clock.Now()}
	_, err = c.db.ExecContext(ctx, "INSERT INTO attachments (id,user_id,media_type,size_bytes,sha256,snippet,created_at) VALUES (?,?,?,?,?,?,?)", id.String(), owner.String(), mediaType, size, hash.String(), snippet, record.CreatedAt.Millisecond())
	return record, sqliteError(err)
}
func (c *Control) Attachment(ctx context.Context, id webapi.AttachmentID) (*AttachmentRecord, error) {
	return attachment(ctx, c.db, id)
}
func attachment(ctx context.Context, db database, id webapi.AttachmentID) (*AttachmentRecord, error) {
	var record AttachmentRecord
	var rawID, owner, hash string
	var ms int64
	err := db.QueryRowContext(ctx, "SELECT id,user_id,media_type,size_bytes,sha256,snippet,created_at FROM attachments WHERE id=?", id.String()).Scan(&rawID, &owner, &record.MediaType, storedCount{"attachments", "size_bytes", &record.SizeBytes}, &hash, &record.Snippet, &ms)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	record.ID, err = webapi.ParseAttachmentID(rawID)
	if err != nil {
		return nil, corrupt("attachments", "id", err)
	}
	record.Owner, err = webapi.ParseUserID(owner)
	if err != nil {
		return nil, corrupt("attachments", "user_id", err)
	}
	record.SHA256, err = core.ParseBlobRef(hash)
	if err != nil {
		return nil, corrupt("attachments", "sha256", err)
	}
	record.CreatedAt, err = instant("attachments", "created_at", ms)
	return &record, err
}
func (c *Control) UploadBlobs(ctx context.Context, owner webapi.UserID) ([]core.BlobRef, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT DISTINCT sha256 FROM attachments WHERE user_id=?", owner.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	blobs := []core.BlobRef{}
	for rows.Next() {
		var text string
		if err = rows.Scan(&text); err != nil {
			return nil, sqliteError(err)
		}
		hash, err := core.ParseBlobRef(text)
		if err != nil {
			return nil, corrupt("attachments", "sha256", err)
		}
		blobs = append(blobs, hash)
	}
	return blobs, sqliteError(rows.Err())
}
