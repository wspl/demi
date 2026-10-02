package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"database/sql"
)

// Summary reads the root's phase, output revision and latest terminal kind
// without loading its transcript. No root means EmptySummary.
func Summary(ctx context.Context, tx *sql.Tx) (SummaryFacts, error) { panic("not written: b-database") }

// HasRoot reports whether the tree has a root, a Fork's destination commit point.
func HasRoot(ctx context.Context, tx *sql.Tx) (bool, error) { panic("not written: b-database") }

// ReadHistory reads root blocks followed by subagents depth first in spawn order.
func ReadHistory(ctx context.Context, tx *sql.Tx) (History, error) { panic("not written: b-database") }
