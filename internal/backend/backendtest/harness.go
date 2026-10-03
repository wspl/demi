package backendtest

import (
	"context"
	"database/sql"
	"errors"
	"net/http"
	"net/netip"
	"net/url"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider/providertest"
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
	// Clock is the shared settable fixture wall clock.
	Clock *providertest.ManualClock
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
	t.Helper()
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	config, err := backend.NewConfig(
		filepath.Join(t.TempDir(), "backend"),
		netip.MustParseAddrPort("127.0.0.1:0"),
		webapi.InstanceModeShared,
		machinesSocket,
	)
	if err != nil {
		return nil, err
	}
	clock := providertest.NewManualClock(core.Timestamp("2026-09-24T08:00:00.000Z"))
	config.Clock = clock
	config.Runners.Ping = nil
	config.Conversations.Titles = false
	config.Lifecycle.RetentionInterval = nil
	config.ModelsDevURL, err = url.Parse("http://127.0.0.1:9/api.json")
	if err != nil {
		return nil, err
	}
	config.ClaudeReleases, err = url.Parse("http://127.0.0.1:9/claude-code-releases")
	if err != nil {
		return nil, err
	}
	return &Harness{Config: config, Clock: clock, Mailbox: &Mailbox{}}, nil
}

// DataDir returns the directory in which starts keep their durable storage.
func (h *Harness) DataDir() string {
	return h.Config.DataDir
}

// ControlDatabase opens an independent connection for corruption scenarios;
// its test cleanup closes it. The caller must not alter the backend's handles.
func (h *Harness) ControlDatabase(ctx context.Context, t testing.TB) (*sql.DB, error) {
	t.Helper()
	db, err := sql.Open("sqlite", filepath.Join(h.DataDir(), "control.sqlite"))
	if err != nil {
		return nil, err
	}
	if _, err := db.ExecContext(ctx, "PRAGMA busy_timeout=5000"); err != nil {
		return nil, errors.Join(err, db.Close())
	}
	t.Cleanup(func() {
		if err := db.Close(); err != nil {
			t.Error(err)
		}
	})
	return db, nil
}

// AddUser inserts a fixture account not created by setup, with a hashed password.
func (h *Harness) AddUser(ctx context.Context, email, password string, role webapi.Role) error {
	address, err := webapi.ParseEmailAddress(email)
	if err != nil {
		return err
	}
	hasher, err := accounts.NewPasswordHasher(ctx)
	if err != nil {
		return err
	}
	hash, err := hasher.Hash(ctx, webapi.Password(password))
	if err != nil {
		return err
	}
	control, err := database.OpenControl(ctx, filepath.Join(h.DataDir(), "control.sqlite"), h.Config.Clock)
	if err != nil {
		return err
	}
	_, createErr := control.CreateUser(ctx, address, hash, role)
	return errors.Join(createErr, control.Close(context.WithoutCancel(ctx)))
}

// WriteWeb writes a fixture web app and sets Config.WebDirectory.
func (h *Harness) WriteWeb(ctx context.Context, files map[string]string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	directory := filepath.Join(filepath.Dir(h.DataDir()), "web")
	if err := os.MkdirAll(directory, 0o755); err != nil {
		return err
	}
	for name, content := range files {
		if err := os.WriteFile(filepath.Join(directory, name), []byte(content), 0o644); err != nil {
			return err
		}
	}
	h.Config.WebDirectory = directory
	return nil
}

// Start starts over the harness data with its configured mode and address;
// NewHarness selects a loopback address and port zero. Test cleanup joins it.
func (h *Harness) Start(ctx context.Context, t testing.TB) (*TestBackend, error) {
	return h.start(ctx, t, h.Config)
}

// StartInMode starts over the same data with mode replacing Config.Mode.
func (h *Harness) StartInMode(ctx context.Context, t testing.TB, mode webapi.InstanceMode) (*TestBackend, error) {
	config := h.Config
	config.Mode = mode
	return h.start(ctx, t, config)
}

// StartAt starts at a previous listener address for runner reconnection tests.
func (h *Harness) StartAt(ctx context.Context, t testing.TB, address netip.AddrPort) (*TestBackend, error) {
	config := h.Config
	config.Address = address
	return h.start(ctx, t, config)
}

// StartSetUp starts and signs in the master account through the setup route.
func (h *Harness) StartSetUp(ctx context.Context, t testing.TB) (*TestBackend, Session, error) {
	b, err := h.Start(ctx, t)
	if err != nil {
		return nil, Session{}, err
	}
	session, err := b.Setup(ctx)
	return b, session, err
}

// start applies fixture-only observation and owns the HTTP connection pool.
func (h *Harness) start(ctx context.Context, t testing.TB, config backend.Config) (*TestBackend, error) {
	t.Helper()
	if h.Objects != nil {
		config.ObserveObjects = h.Objects.Observe
	}
	b, err := Start(ctx, t, config)
	if err != nil {
		return nil, err
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.Proxy = nil
	client := &http.Client{Transport: transport}
	t.Cleanup(client.CloseIdleConnections)
	return &TestBackend{URL: "http://" + b.LocalAddr().String(), Backend: b, HTTP: client}, nil
}
