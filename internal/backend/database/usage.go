package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/webapi"
)

// AppendUsage writes the row, timed now.
func (c *ControlService) AppendUsage(ctx context.Context, row UsageRow) error {
	panic("not written: b-database")
}

// UsageTotals returns the user's totals.
func (c *ControlService) UsageTotals(ctx context.Context, user webapi.UserID) ([]webapi.UsageGroup, error) {
	panic("not written: b-database")
}

// InstanceUsage returns every account's totals, the accounts in the order they were created,
// read in one transaction.
func (c *ControlService) InstanceUsage(ctx context.Context) ([]webapi.UserUsage, error) {
	panic("not written: b-database")
}
