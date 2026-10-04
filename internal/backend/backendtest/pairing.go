package backendtest

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/fsnotify/fsnotify"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapiproto"
)

// Paired is a claimed device and its test-owned runner process.
type Paired struct {
	// Runner owns the paired device runner process.
	Runner *remotehosttest.RunnerProcess
	// Device is the validated claimed device.
	Device webapiproto.DeviceDTO
}

// ID returns the claimed device's ID.
func (p *Paired) ID() webapiproto.DeviceID {
	return p.Device.ID
}

// Token waits until the runner has durably received its device token.
func (p *Paired) Token(ctx context.Context) (string, error) {
	return StoredToken(ctx, p.Runner)
}

// Pair starts, claims and waits for a new named device and its stored token.
// The runner fixture registers process cleanup before it starts the child.
func (b *TestBackend) Pair(ctx context.Context, t testing.TB, session *Session, name string) (*Paired, error) {
	options := remotehosttest.DefaultRunnerProcessOptions()
	options.Name = name
	runner, err := remotehosttest.StartRunnerProcess(ctx, t, b.URL, options)
	if err != nil {
		return nil, err
	}
	code, err := runner.PairingCode(ctx, 0)
	if err != nil {
		return nil, err
	}
	body, err := contract.EncodeJSON(webapiproto.Claim{Code: code})
	if err != nil {
		return nil, err
	}
	answer, err := b.Post(ctx, "/api/devices/claim", session, body)
	if err != nil {
		return nil, err
	}
	if answer.Status != http.StatusCreated {
		return nil, fmt.Errorf("claim: HTTP %d: %s", answer.Status, answer.Body)
	}
	claimed, err := webapiproto.DecodeClaimedDevice(answer.Body)
	if err != nil {
		return nil, err
	}
	if err := b.UntilOnline(ctx, session, claimed.Device.ID, true); err != nil {
		return nil, err
	}
	if _, err := StoredToken(ctx, runner); err != nil {
		return nil, err
	}
	return &Paired{Runner: runner, Device: claimed.Device}, nil
}

// StoredToken waits for filesystem publication of the runner's token. The
// directory watch is installed before reading, so atomic rename cannot be lost.
func StoredToken(ctx context.Context, runner *remotehosttest.RunnerProcess) (token string, err error) {
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		return "", err
	}
	defer func() {
		err = errors.Join(err, watcher.Close())
	}()
	if err := watcher.Add(runner.StateDir()); err != nil {
		return "", err
	}
	path := filepath.Join(runner.StateDir(), "runner-token")
	for {
		data, err := os.ReadFile(path)
		if err == nil {
			return strings.TrimSpace(string(data)), nil
		}
		if !errors.Is(err, os.ErrNotExist) {
			return "", err
		}
		select {
		case <-ctx.Done():
			return "", ctx.Err()
		case err := <-watcher.Errors:
			return "", err
		case <-watcher.Events:
		}
	}
}
