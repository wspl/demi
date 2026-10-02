package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// ControlService owns the deployment's control records and single writer.
// Each operation owns one transaction. Close drains admitted operations.
type ControlService struct{}

// OpenControl opens the database, and gives a new one its schema.
func OpenControl(ctx context.Context, path string, clock core.Clock) (*ControlService, error) {
	panic("not written: b-database")
}

// MaxWriters is the most writer connections open at once.
const MaxWriters = 64

// ConversationStores owns lazy conversation handles and the bounded writer LRU.
// Cold reads use independently bounded read-only connections and create no files.
type ConversationStores struct{}

// ConversationDB is one conversation's database, as a handle that stays valid
// while its writer closes and opens again.
type ConversationDB struct{}

// OpenConversations opens the directory, creating it if missing, with at most
// maxWriters writers open at once. maxWriters must be in [1, MaxWriters].
func OpenConversations(ctx context.Context, directory string, maxWriters int) (*ConversationStores, error) {
	panic("not written: b-database")
}

// DB returns the stable lazy database handle for conversation.
func (s *ConversationStores) DB(conversation webapi.ConversationID) *ConversationDB {
	panic("not written: b-database")
}

// Read runs work in one read-only transaction; false means no database exists.
// The transaction belongs to the store and must not escape work.
func (s *ConversationStores) Read(ctx context.Context, conversation webapi.ConversationID, work func(context.Context, *sql.Tx) error) (bool, error) {
	panic("not written: b-database")
}

// Close drains operations and closes all connections, joining all close errors.
// Later calls and reads fail with ErrClosed.
func (s *ConversationStores) Close(ctx context.Context) error { panic("not written: b-database") }

// Call runs work in one writer transaction, creating the database if needed.
// The store commits on success and rolls back on failure. The transaction must
// not escape work. Work must not wait on a model or network operation.
func (d *ConversationDB) Call(ctx context.Context, work func(context.Context, *sql.Tx) error) error {
	panic("not written: b-database")
}

// Read runs work in one read-only transaction; false means no database exists.
// The transaction belongs to the store and must not escape work.
func (d *ConversationDB) Read(ctx context.Context, work func(context.Context, *sql.Tx) error) (bool, error) {
	panic("not written: b-database")
}
