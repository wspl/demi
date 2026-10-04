package database

import (
	"context"
	"database/sql"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// AppendUsage writes the row, timed now.
func (c *ControlService) AppendUsage(ctx context.Context, row UsageRow) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, now types.Timestamp) error {
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
func (c *ControlService) UsageTotals(ctx context.Context, user webapiproto.UserID) ([]webapiproto.UsageGroup, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]webapiproto.UsageGroup, error) {
			return usageTotals(ctx, tx, user)
		},
	)
}

// InstanceUsage returns every account's totals, the accounts in the order they were created,
// read in one transaction.
func (c *ControlService) InstanceUsage(ctx context.Context) ([]webapiproto.UserUsage, error) {
	return controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]webapiproto.UserUsage, error) {
			users, err := queryRecords(
				ctx,
				tx,
				"users",
				"SELECT id,email FROM users ORDER BY created_at,rowid",
				func(r *storedRow) webapiproto.UserUsage {
					return webapiproto.UserUsage{
						UserID: checked(r, "id", webapiproto.ParseUserID),
						Email:  checked(r, "email", webapiproto.ParseEmailAddress),
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
		},
	)
}

func usageTotals(ctx context.Context, tx *sql.Tx, user webapiproto.UserID) ([]webapiproto.UsageGroup, error) {
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
		func(r *storedRow) webapiproto.UsageGroup {
			return webapiproto.UsageGroup{
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
