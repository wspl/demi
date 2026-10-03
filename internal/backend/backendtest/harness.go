package backendtest

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"database/sql"
	"net/netip"
	"testing"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
	"github.com/wspl/demi/internal/webapi"
)

const (
	// MasterEmail is the harness master account's email address.
	MasterEmail = "master@example.test"
	// MasterPassword is the harness master account's password.
	MasterPassword = "master-pass-1"
	// SessionCookie is the browser session cookie's name.
	SessionCookie = "demi_session"
)

// Harness is what a test backend starts from; it can start several times
// over the same data directory. Its test owns the temporary files and workers.
// Set Config fields before starting; change them only between starts.
type Harness struct {
	// Config holds typed replacements, including Clock, families, timings,
	// plugins, native catalog, mail, public URL and machine-manager socket.
	Config backend.Config
	// Mailbox captures verification codes when Config.AccountMail names it.
	Mailbox *Mailbox
	// Objects optionally counts object operations across all starts.
	Objects *blobstest.ObjectCounts
}

// NewHarness creates temporary data and default test settings with no real
// vendor endpoints, title requests, runner pings or scheduled retention.
// machinesSocket names the scripted or real manager owned by the caller.
// The caller can supply providertest.ManualClock through Config.Clock.
func NewHarness(ctx context.Context, t testing.TB, machinesSocket string) (*Harness, error) {
	panic("not written: b-backend")
}

// DataDir returns the directory in which starts keep their durable storage.
func (h *Harness) DataDir() string { panic("not written: b-backend") }

// ControlDatabase opens an independent connection for corruption scenarios;
// its test cleanup closes it. The caller must not alter the backend's handles.
func (h *Harness) ControlDatabase(ctx context.Context, t testing.TB) (*sql.DB, error) {
	panic("not written: b-backend")
}

// AddUser inserts a fixture account not created by setup, with a hashed password.
func (h *Harness) AddUser(ctx context.Context, email, password string, role webapi.Role) error {
	panic("not written: b-backend")
}

// WriteWeb writes a fixture web app and sets Config.WebDirectory.
func (h *Harness) WriteWeb(ctx context.Context, files map[string]string) error {
	panic("not written: b-backend")
}

// Start starts over the harness data with its configured mode and address;
// NewHarness selects a loopback address and port zero. Test cleanup joins it.
func (h *Harness) Start(ctx context.Context, t testing.TB) (*TestBackend, error) {
	panic("not written: b-backend")
}

// StartInMode starts over the same data with mode replacing Config.Mode.
func (h *Harness) StartInMode(ctx context.Context, t testing.TB, mode webapi.InstanceMode) (*TestBackend, error) {
	panic("not written: b-backend")
}

// StartAt starts at a previous listener address for runner reconnection tests.
func (h *Harness) StartAt(ctx context.Context, t testing.TB, address netip.AddrPort) (*TestBackend, error) {
	panic("not written: b-backend")
}

// StartSetUp starts and signs in the master account through the setup route.
func (h *Harness) StartSetUp(ctx context.Context, t testing.TB) (*TestBackend, Session, error) {
	panic("not written: b-backend")
}
