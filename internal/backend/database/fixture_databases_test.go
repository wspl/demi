package database

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/core"
)

// The fixture databases were written through the backend's public API, not reconstructed SQL.
func TestFixtureDatabasesReadUnchanged(t *testing.T) {
	copyFixture := func(source, destination string) [32]byte {
		t.Helper()
		data, err := os.ReadFile(source)
		require(t, err)
		require(t, os.WriteFile(destination, data, 0o600))
		return sha256.Sum256(data)
	}
	directory := t.TempDir()
	controlPath := filepath.Join(directory, "control.sqlite")
	before := copyFixture("testdata/control.sqlite", controlPath)
	control, err := OpenControl(t.Context(), controlPath, core.SystemClock{})
	require(t, err)
	users, err := control.Users(t.Context())
	require(t, err)
	equal(t, 1, len(users))
	equal(t, "master@example.test", string(users[0].Email))
	account, ok, err := control.Account(t.Context(), users[0].ID)
	require(t, err)
	if !ok || account.PasswordHash.Text() == "" {
		t.Fatal("fixture account missing")
	}
	conversations, err := control.Conversations(t.Context(), users[0].ID, false)
	require(t, err)
	equal(t, 1, len(conversations))
	equal(t, conversation(1), conversations[0].ID)
	equal(t, "New conversation", conversations[0].Title)
	require(t, control.Close(context.Background()))
	after, err := os.ReadFile(controlPath)
	require(t, err)
	equal(t, before, sha256.Sum256(after))
	path := filepath.Join(directory, string(conversation(1))+".sqlite")
	before = copyFixture(filepath.Join("testdata/conversations", string(conversation(1))+".sqlite"), path)
	stores, err := OpenConversations(t.Context(), directory, 1)
	require(t, err)
	found, err := stores.DB(conversation(1)).Read(t.Context(), func(ctx context.Context, tx *sql.Tx) error {
		sequences, err := Sequences(ctx, tx)
		if err != nil {
			return err
		}
		equal(t, []SequenceNext{{Sequence: core.SequenceCommand, Next: 2}}, sequences)
		return nil
	})
	require(t, err)
	equal(t, true, found)
	require(t, stores.Close(context.Background()))
	after, err = os.ReadFile(path)
	require(t, err)
	equal(t, before, sha256.Sum256(after))
}
