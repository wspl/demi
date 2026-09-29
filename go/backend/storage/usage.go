package storage

import (
	"context"
	"math"

	"github.com/google/uuid"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type UsageRow struct {
	User         webapi.UserID
	Conversation webapi.ConversationID
	Provider     webapi.ProviderID
	Model        string
	Usage        core.TokenUsage
}

func (c *Control) AppendUsage(ctx context.Context, row UsageRow) error {
	tokens := []uint64{row.Usage.InputTokens, row.Usage.OutputTokens, row.Usage.CacheReadTokens, row.Usage.CacheWriteTokens}
	counts := make([]int64, len(tokens))
	for i, n := range tokens {
		counts[i] = int64(min(n, math.MaxInt64))
	}
	_, err := c.db.ExecContext(ctx, `INSERT INTO usage_ledger (id,user_id,conversation_id,provider_id,model_id,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,created_at) VALUES (?,?,?,?,?,?,?,?,?,?)`, uuid.NewString(), row.User.String(), row.Conversation.String(), row.Provider.String(), row.Model, counts[0], counts[1], counts[2], counts[3], c.clock.Now().Millisecond())
	return sqliteError(err)
}
func usageTotals(ctx context.Context, db database, user webapi.UserID) ([]webapi.UsageGroup, error) {
	rows, err := db.QueryContext(ctx, `SELECT provider_id,model_id,COUNT(*),SUM(input_tokens),SUM(output_tokens),SUM(cache_read_tokens),SUM(cache_write_tokens) FROM usage_ledger WHERE user_id=? GROUP BY provider_id,model_id ORDER BY MIN(created_at),MIN(rowid)`, user.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	groups := []webapi.UsageGroup{}
	for rows.Next() {
		var group webapi.UsageGroup
		var counts [5]int64
		if err = rows.Scan(&group.ProviderID, &group.ModelID, &counts[0], &counts[1], &counts[2], &counts[3], &counts[4]); err != nil {
			return nil, sqliteError(err)
		}
		names := []string{"requests", "input_tokens", "output_tokens", "cache_read_tokens", "cache_write_tokens"}
		for i, n := range counts {
			if n < 0 {
				return nil, &CorruptError{"usage_ledger", names[i], "out of range integral type conversion attempted"}
			}
		}
		group.Requests = uint64(counts[0])
		group.InputTokens = uint64(counts[1])
		group.OutputTokens = uint64(counts[2])
		group.CacheReadTokens = uint64(counts[3])
		group.CacheWriteTokens = uint64(counts[4])
		groups = append(groups, group)
	}
	return groups, sqliteError(rows.Err())
}
func (c *Control) UsageTotals(ctx context.Context, user webapi.UserID) ([]webapi.UsageGroup, error) {
	return usageTotals(ctx, c.db, user)
}
func (c *Control) InstanceUsage(ctx context.Context) ([]webapi.UserUsage, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	rows, err := tx.QueryContext(ctx, "SELECT "+userColumns+" FROM users ORDER BY created_at,rowid")
	if err != nil {
		return nil, sqliteError(err)
	}
	users := []webapi.UserUsage{}
	for rows.Next() {
		a, readErr := scanUser(rows, false)
		if readErr != nil {
			rows.Close()
			return nil, readErr
		}
		users = append(users, webapi.UserUsage{UserID: a.User.ID, Email: a.User.Email})
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return nil, sqliteError(err)
	}
	for i := range users {
		users[i].Totals, err = usageTotals(ctx, tx, users[i].UserID)
		if err != nil {
			return nil, err
		}
	}
	return users, sqliteError(tx.Commit())
}
