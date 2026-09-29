package storage

import (
	"context"
	"database/sql"
	"slices"
	"strings"

	"github.com/wspl/demi/go/core"
)

type BlobRefRow struct {
	Part   int
	Blob   core.BlobRef
	Holder string
	At     core.Timestamp
}

func contentReferences(content []core.UserContentBlock) []core.BlobRef {
	refs := []core.BlobRef{}
	for _, part := range content {
		var source core.MediaSource
		switch v := part.(type) {
		case core.UserContentBlockImage:
			source = v.Source
		case core.UserContentBlockVideo:
			source = v.Source
		case core.UserContentBlockDocument:
			if ref, ok := v.Source.(core.DocumentSourceRef); ok {
				refs = append(refs, ref.Ref)
			}
		}
		if ref, ok := source.(core.MediaSourceRef); ok {
			refs = append(refs, ref.Ref)
		}
	}
	return refs
}
func BlockReferences(block core.Block) []BlobRefRow {
	rows := []BlobRefRow{}
	at := core.BlockCreatedAt(block)
	add := func(blob core.BlobRef, holder string) { rows = append(rows, BlobRefRow{len(rows), blob, holder, at}) }
	switch b := block.(type) {
	case core.BlockUser:
		for _, blob := range contentReferences(b.Content) {
			add(blob, "message")
		}
	case core.BlockSteer:
		for _, blob := range contentReferences(b.Content) {
			add(blob, "message")
		}
	case core.BlockToolCall:
		for _, part := range b.Output {
			var source core.ToolMediaSource
			switch v := part.(type) {
			case core.ToolResultContentBlockImage:
				source = v.Source
			case core.ToolResultContentBlockVideo:
				source = v.Source
			}
			if ref, ok := source.(core.ToolMediaSourceRef); ok {
				add(ref.Ref, "tool_result")
			}
		}
		if b.View != nil {
			if view, ok := (*b.View).(core.ToolViewShell); ok && view.Files != nil {
				for _, file := range *view.Files {
					for _, edit := range file.Edits {
						if edit.Copies != nil {
							add(edit.Copies.Original, "edit_copy")
							add(edit.Copies.Modified, "edit_copy")
						}
					}
				}
			}
		}
	}
	return rows
}
func queryBlobs(ctx context.Context, db database, query string, args ...any) ([]core.BlobRef, error) {
	rows, err := db.QueryContext(ctx, query, args...)
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
		blob, err := core.ParseBlobRef(text)
		if err != nil {
			return nil, corrupt("blob_refs", "blob", err)
		}
		blobs = append(blobs, blob)
	}
	return blobs, sqliteError(rows.Err())
}
func writeBlock(ctx context.Context, tx *sql.Tx, node core.NodeID, index int, block core.Block, touched *[]core.BlobRef) error {
	text, err := core.Encode(block)
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO blocks(node_id,idx,block) VALUES (?,?,?) ON CONFLICT(node_id,idx) DO UPDATE SET block=excluded.block", node.String(), index, string(text)); err != nil {
		return sqliteError(err)
	}
	removed, err := queryBlobs(ctx, tx, "DELETE FROM blob_refs WHERE node_id=? AND idx=? RETURNING blob", node.String(), index)
	if err != nil {
		return err
	}
	*touched = append(*touched, removed...)
	for _, row := range BlockReferences(block) {
		if _, err = tx.ExecContext(ctx, "INSERT INTO blob_refs(node_id,idx,part,blob,holder,at) VALUES (?,?,?,?,?,?)", node.String(), index, row.Part, row.Blob.String(), row.Holder, row.At.Millisecond()); err != nil {
			return sqliteError(err)
		}
		*touched = append(*touched, row.Blob)
	}
	return nil
}
func (d *ConversationDB) References(ctx context.Context) ([]core.BlobRef, error) {
	refs := map[core.BlobRef]bool{}
	_, err := d.read(ctx, func(tx *sql.Tx) error {
		blobs, err := queryBlobs(ctx, tx, "SELECT blob FROM blob_refs UNION SELECT blob FROM command_outputs WHERE blob IS NOT NULL")
		if err != nil {
			return err
		}
		for _, blob := range blobs {
			refs[blob] = true
		}
		rows, err := tx.QueryContext(ctx, "SELECT state FROM nodes")
		if err != nil {
			return sqliteError(err)
		}
		defer rows.Close()
		for rows.Next() {
			var text string
			if err = rows.Scan(&text); err != nil {
				return sqliteError(err)
			}
			state, err := decodeState(text)
			if err != nil {
				return err
			}
			for _, message := range state.Queue {
				for _, blob := range contentReferences(message.Content) {
					refs[blob] = true
				}
			}
		}
		return sqliteError(rows.Err())
	})
	if err != nil {
		return nil, err
	}
	result := make([]core.BlobRef, 0, len(refs))
	for blob := range refs {
		result = append(result, blob)
	}
	slices.SortFunc(result, func(a, b core.BlobRef) int { return strings.Compare(a.String(), b.String()) })
	return result, nil
}
