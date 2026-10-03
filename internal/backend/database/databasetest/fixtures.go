package databasetest

import (
	"context"
	"database/sql"
	"errors"
	"path/filepath"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Control opens a control database in a test directory and registers cleanup.
// The test owns every resource; clock supplies record timestamps.
func Control(ctx context.Context, t testing.TB, clock core.Clock) *database.ControlService {
	t.Helper()
	path := filepath.Join(t.TempDir(), "control.sqlite")
	control, err := database.OpenControl(ctx, path, clock)
	if err != nil {
		t.Fatal(err)
	}
	controlPaths.Store(control, path)
	t.Cleanup(func() {
		controlPaths.Delete(control)
		if err := control.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return control
}

// Master creates the fixture's master@example.test account with the Rust test PHC hash.
func Master(ctx context.Context, t testing.TB, control *database.ControlService) webapi.UserDTO {
	t.Helper()
	hash, err := database.ParsePasswordHash(
		"$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$0mUbQTTMhhaEBFGMq7WTZxOlVoS9sY3qVqLiV7Q1Izo",
	)
	if err != nil {
		t.Fatal(err)
	}
	email, err := webapi.ParseEmailAddress("master@example.test")
	if err != nil {
		t.Fatal(err)
	}
	master, err := control.CreateMaster(ctx, email, hash)
	if errors.Is(err, database.ErrAlreadySetUp) {
		t.Fatal("fixture master already exists")
	}
	if err != nil {
		t.Fatal(err)
	}

	return master
}

// Execute runs SQL with text parameters behind the control service's back, for
// black-box corruption tests. It fails the test on an execution error.
func Execute(
	ctx context.Context,
	t testing.TB,
	control *database.ControlService,
	statement string,
	parameters ...string,
) {
	t.Helper()
	value, ok := controlPaths.Load(control)
	if !ok {
		t.Fatal("Execute requires a control opened by databasetest.Control")
	}
	path, ok := value.(string)
	if !ok {
		t.Fatal("invalid fixture control path")
	}
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := db.Close(); err != nil {
			t.Error(err)
		}
	}()
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := tx.Rollback(); err != nil && !errors.Is(err, sql.ErrTxDone) {
			t.Error(err)
		}
	}()
	args := make([]any, len(parameters))
	for i, p := range parameters {
		args[i] = p
	}
	if _, err := tx.ExecContext(ctx, statement, args...); err != nil {
		t.Fatal(err)
	}
	if err := tx.Commit(); err != nil {
		t.Fatal(err)
	}
}

// Conversations opens a temporary conversation directory and registers cleanup.
func Conversations(ctx context.Context, t testing.TB, maxWriters int) *database.ConversationStores {
	t.Helper()
	stores, err := database.OpenConversations(ctx, t.TempDir(), maxWriters)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := stores.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return stores
}

// CommitHold stops checkpoints after writing rows and before committing.
// Release or test cleanup releases all waiting commits.
type CommitHold = database.CommitHold

// HoldCommits holds checkpoints from now until Release and registers test cleanup.
func HoldCommits(t testing.TB, stores *database.ConversationStores) *CommitHold {
	t.Helper()
	hold := stores.HoldCommits()
	t.Cleanup(hold.Release)
	return hold
}

// Key returns a deterministic 32-byte test key for fixtures storing sealed records.
// It is test data and must never protect production credentials.
func Key() [32]byte {
	var key [32]byte
	for i := range key {
		key[i] = byte(i)
	}
	return key
}

// controlPaths keeps corruption access private to each fixture lifetime.
var controlPaths sync.Map
