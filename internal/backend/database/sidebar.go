package database

import (
	"context"
	"database/sql"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// ReorderConversations moves the user's conversation `moved` before `before`, or to the end,
// within its partition (`storage.md` § Control records): the
// conversations that are not archived, pinned as it is, and in the same
// workspace or in none. False, writing nothing, when either is not in
// that partition, an archived one included.
func (c *ControlService) ReorderConversations(
	ctx context.Context,
	user webapi.UserID,
	moved webapi.ConversationID,
	before *webapi.ConversationID,
) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (bool, error) {
		type partition struct {
			archived, pinned bool
			workspace        *string
		}
		p, err := queryRecord(
			ctx,
			tx,
			"conversations",
			"SELECT archived,pinned,target_workspace_id FROM conversations WHERE id = ? AND user_id = ?",
			func(r *storedRow) partition {
				return partition{r.boolean("archived"), r.boolean("pinned"), r.optionalText("target_workspace_id")}
			},
			moved,
			user,
		)
		if err != nil || p == nil {
			return false, err
		}
		if p.archived {
			return false, nil
		}
		peers, err := queryRecords(
			ctx,
			tx,
			"conversations",
			`SELECT id
FROM conversations
WHERE user_id = ? AND archived = 0 AND pinned = ? AND target_workspace_id IS ?
ORDER BY sort_order,id`,
			func(r *storedRow) string { return r.text("id") },
			user,
			p.pinned,
			p.workspace,
		)
		if err != nil {
			return false, err
		}
		var b *string
		if before != nil {
			v := string(*before)
			b = &v
		}
		return writeOrder(ctx, tx, "conversations", peers, string(moved), b)
	})
}

// ReorderWorkspaces moves the user's workspace `moved` before `before`, or to the end;
// false when either is not one of the user's workspaces.
func (c *ControlService) ReorderWorkspaces(
	ctx context.Context,
	user webapi.UserID,
	moved webapi.WorkspaceID,
	before *webapi.WorkspaceID,
) (bool, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (bool, error) {
		peers, err := queryRecords(
			ctx,
			tx,
			"workspaces",
			"SELECT id FROM workspaces WHERE user_id = ? ORDER BY sort_order,id",
			func(r *storedRow) string { return r.text("id") },
			user,
		)
		if err != nil {
			return false, err
		}
		var b *string
		if before != nil {
			v := string(*before)
			b = &v
		}
		return writeOrder(ctx, tx, "workspaces", peers, string(moved), b)
	})
}

func writeOrder(
	ctx context.Context,
	tx *sql.Tx,
	table string,
	peers []string,
	moved string,
	before *string,
) (bool, error) {
	from := slices.IndexFunc(peers, func(s string) bool { return strings.EqualFold(s, moved) })
	if from < 0 {
		return false, nil
	}
	if before != nil && !slices.ContainsFunc(peers, func(s string) bool { return strings.EqualFold(s, *before) }) {
		return false, nil
	}
	row := peers[from]
	peers = slices.Delete(peers, from, from+1)
	to := len(peers)
	if before != nil {
		to = slices.IndexFunc(peers, func(s string) bool { return strings.EqualFold(s, *before) })
		if to < 0 {
			to = from
		}
	}
	peers = slices.Insert(peers, to, row)
	for position, peer := range peers {
		if err := execSQL(ctx, tx, "UPDATE "+table+" SET sort_order = ? WHERE id = ?", position, peer); err != nil {
			return false, err
		}
	}
	return true, nil
}
