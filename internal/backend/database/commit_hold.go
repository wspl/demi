package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"database/sql"
)

// CommitPoint is where a checkpoint transaction commits, immediately unless
// a test holds commits. Production callers use the store's transaction owner.
type CommitPoint struct{}

// CommitPoint captures the checkpoint commit point for this database.
func (d *ConversationDB) CommitPoint() CommitPoint { panic("not written: b-database") }

// Commit commits tx after any test hold is released. Its caller owns rollback
// on failure. The checkpoint owns its commit-guard check.
func (p CommitPoint) Commit(ctx context.Context, tx *sql.Tx) error { panic("not written: b-database") }

// CommitHold stops checkpoints after their rows are written and before commit.
// The owner must release the hold on every path; databasetest registers cleanup.
type CommitHold struct{}

// HoldCommits holds checkpoints from now until the returned hold is released.
// Tests should use databasetest.HoldCommits, which registers cleanup.
func (s *ConversationStores) HoldCommits() *CommitHold { panic("not written: b-database") }

// UntilWaiting waits until count checkpoint commits are held, or ctx is canceled.
func (h *CommitHold) UntilWaiting(ctx context.Context, count int) error {
	panic("not written: b-database")
}

// Release lets waiting and subsequent checkpoint commits through.
// Repeated calls are harmless.
func (h *CommitHold) Release() { panic("not written: b-database") }
