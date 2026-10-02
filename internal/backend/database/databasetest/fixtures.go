package databasetest

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Control opens a control database in a test directory and registers cleanup.
// The test owns every resource; clock supplies record timestamps.
func Control(ctx context.Context, t testing.TB, clock core.Clock) *database.ControlService {
	panic("not written: b-database")
}

// Master creates the fixture's master@example.test account with the Rust test PHC hash.
func Master(ctx context.Context, t testing.TB, control *database.ControlService) webapi.UserDTO {
	panic("not written: b-database")
}

// Execute runs SQL with text parameters behind the control service's back, for
// black-box corruption tests. It fails the test on an execution error.
func Execute(ctx context.Context, t testing.TB, control *database.ControlService, statement string, parameters ...string) {
	panic("not written: b-database")
}

// Conversations opens a temporary conversation directory and registers cleanup.
func Conversations(ctx context.Context, t testing.TB, maxWriters int) *database.ConversationStores {
	panic("not written: b-database")
}

// CommitHold stops checkpoints after writing rows and before committing.
// Release or test cleanup releases all waiting commits.
type CommitHold = database.CommitHold

// HoldCommits holds checkpoints from now until Release and registers test cleanup.
func HoldCommits(t testing.TB, stores *database.ConversationStores) *CommitHold {
	panic("not written: b-database")
}

// Key returns a deterministic 32-byte test key for fixtures storing sealed records.
// It is test data and must never protect production credentials.
func Key() [32]byte { panic("not written: b-database") }
