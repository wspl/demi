package database

import (
	"context"
	"database/sql"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// AppendUsage writes the row, timed now.
func (c *ControlService) AppendUsage(ctx context.Context, row UsageRow) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) error {
		at, err := now.Millisecond()
		if err != nil {
			return err
		}
		return execSQL(
			ctx,
			tx,
			`INSERT INTO usage_ledger (
    id,
    user_id,
    conversation_id,
    provider_id,
    model_id,
    input_tokens,
    output_tokens,
    cache_read_tokens,
    cache_write_tokens,
    created_at
)
VALUES (?,?,?,?,?,?,?,?,?,?)`,
			uuid.NewString(),
			row.User,
			row.Conversation,
			row.Provider,
			row.Model,
			integer(row.Usage.InputTokens),
			integer(row.Usage.OutputTokens),
			integer(row.Usage.CacheReadTokens),
			integer(row.Usage.CacheWriteTokens),
			at,
		)
	})
}

// UsageTotals returns the user's totals.
func (c *ControlService) UsageTotals(ctx context.Context, user webapi.UserID) ([]webapi.UsageGroup, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) ([]webapi.UsageGroup, error) {
		return usageTotals(ctx, tx, user)
	})
}

// InstanceUsage returns every account's totals, the accounts in the order they were created,
// read in one transaction.
func (c *ControlService) InstanceUsage(ctx context.Context) ([]webapi.UserUsage, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) ([]webapi.UserUsage, error) {
		users, err := queryRecords(
			ctx,
			tx,
			"users",
			"SELECT id,email FROM users ORDER BY created_at,rowid",
			func(r *storedRow) webapi.UserUsage {
				return webapi.UserUsage{
					UserID: checked(r, "id", webapi.ParseUserID),
					Email:  checked(r, "email", webapi.ParseEmailAddress),
				}
			},
		)
		if err != nil {
			return nil, err
		}
		for i := range users {
			users[i].Totals, err = usageTotals(ctx, tx, users[i].UserID)
			if err != nil {
				return nil, err
			}
		}
		return users, nil
	})
}

func usageTotals(ctx context.Context, tx *sql.Tx, user webapi.UserID) ([]webapi.UsageGroup, error) {
	return queryRecords(
		ctx,
		tx,
		"usage_ledger",
		`SELECT
    provider_id,
    model_id,
    COUNT(*) AS requests,
    SUM(input_tokens) AS input_tokens,
    SUM(output_tokens) AS output_tokens,
    SUM(cache_read_tokens) AS cache_read_tokens,
    SUM(cache_write_tokens) AS cache_write_tokens
FROM usage_ledger
WHERE user_id = ?
GROUP BY provider_id,model_id
ORDER BY MIN(created_at),MIN(rowid)`,
		func(r *storedRow) webapi.UsageGroup {
			return webapi.UsageGroup{
				ProviderID:       r.text("provider_id"),
				ModelID:          r.text("model_id"),
				Requests:         r.count("requests"),
				InputTokens:      r.count("input_tokens"),
				OutputTokens:     r.count("output_tokens"),
				CacheReadTokens:  r.count("cache_read_tokens"),
				CacheWriteTokens: r.count("cache_write_tokens"),
			}
		},
		user,
	)
}
