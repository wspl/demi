package storage

import (
	"context"
	"database/sql"
	"errors"
	"slices"
	"strings"

	"github.com/wspl/demi/go/webapi"
)

func writeOrder(ctx context.Context, db database, table string, peers []string, moved string, before *string) (bool, error) {
	find := func(id string) int {
		return slices.IndexFunc(peers, func(peer string) bool { return strings.EqualFold(peer, id) })
	}
	from := find(moved)
	if from < 0 || (before != nil && find(*before) < 0) {
		return false, nil
	}
	row := peers[from]
	peers = slices.Delete(peers, from, from+1)
	to := len(peers)
	if before != nil {
		to = find(*before)
		if to < 0 {
			to = from
		}
	}
	peers = slices.Insert(peers, to, row)
	for i, id := range peers {
		if _, err := db.ExecContext(ctx, "UPDATE "+table+" SET sort_order=? WHERE id=?", i, id); err != nil {
			return false, sqliteError(err)
		}
	}
	return true, nil
}
func sidebarPeers(ctx context.Context, db database, query string, args ...any) ([]string, error) {
	rows, err := db.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	peers := []string{}
	for rows.Next() {
		var id string
		if err = rows.Scan(&id); err != nil {
			return nil, sqliteError(err)
		}
		peers = append(peers, id)
	}
	return peers, sqliteError(rows.Err())
}
func (c *Control) ReorderWorkspaces(ctx context.Context, user webapi.UserID, moved webapi.WorkspaceID, before *webapi.WorkspaceID) (bool, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return false, sqliteError(err)
	}
	defer tx.Rollback()
	peers, err := sidebarPeers(ctx, tx, "SELECT id FROM workspaces WHERE user_id=? ORDER BY sort_order,id", user.String())
	if err != nil {
		return false, err
	}
	var next *string
	if before != nil {
		text := before.String()
		next = &text
	}
	ok, err := writeOrder(ctx, tx, "workspaces", peers, moved.String(), next)
	if err != nil || !ok {
		return ok, err
	}
	return true, sqliteError(tx.Commit())
}
func (c *Control) ReorderConversations(ctx context.Context, user webapi.UserID, moved webapi.ConversationID, before *webapi.ConversationID) (bool, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return false, sqliteError(err)
	}
	defer tx.Rollback()
	var archived, pinned bool
	var workspace *string
	err = tx.QueryRowContext(ctx, "SELECT archived,pinned,target_workspace_id FROM conversations WHERE id=? AND user_id=?", moved.String(), user.String()).Scan(&archived, &pinned, &workspace)
	if errors.Is(err, sql.ErrNoRows) || archived {
		return false, nil
	}
	if err != nil {
		return false, sqliteError(err)
	}
	peers, err := sidebarPeers(ctx, tx, "SELECT id FROM conversations WHERE user_id=? AND archived=0 AND pinned=? AND target_workspace_id IS ? ORDER BY sort_order,id", user.String(), pinned, workspace)
	if err != nil {
		return false, err
	}
	var next *string
	if before != nil {
		text := before.String()
		next = &text
	}
	ok, err := writeOrder(ctx, tx, "conversations", peers, moved.String(), next)
	if err != nil || !ok {
		return ok, err
	}
	return true, sqliteError(tx.Commit())
}
