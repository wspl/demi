package runner

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// runnerState keeps validated registration records within the locked installation.
type runnerState struct{ root string }

func (s runnerState) config() (runnerConfig, bool, error) {
	data, err := os.ReadFile(filepath.Join(s.root, "runner.json"))
	if errors.Is(err, os.ErrNotExist) {
		return runnerConfig{}, false, nil
	}
	if err != nil {
		return runnerConfig{}, false, err
	}
	value, err := decodeRunnerConfig(data)
	if err != nil {
		return runnerConfig{}, false, err
	}
	return value, true, nil
}

func (s runnerState) writeConfig(ctx context.Context, config runnerConfig) error {
	data, err := config.MarshalJSON()
	if err != nil {
		return err
	}
	return process.WritePrivate(ctx, filepath.Join(s.root, "runner.json"), data)
}

func (s runnerState) token() (*runnerwire.DeviceToken, error) {
	data, err := os.ReadFile(filepath.Join(s.root, "runner-token"))
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	if !utf8.Valid(data) {
		return nil, errors.New("invalid UTF-8 in device token")
	}
	token, err := runnerwire.ParseDeviceToken(strings.TrimSpace(string(data)))
	return &token, err
}

func (s runnerState) writeToken(ctx context.Context, token runnerwire.DeviceToken) error {
	return process.WritePrivate(ctx, filepath.Join(s.root, "runner-token"), []byte(token.Expose()+"\n"))
}

func (s runnerState) active() (activeRunner, error) {
	data, err := os.ReadFile(filepath.Join(s.root, "active.json"))
	if err != nil {
		return activeRunner{}, err
	}
	return decodeActiveRunner(data)
}

func (l *installationLease) publish(ctx context.Context, state runnerState, active activeRunner) error {
	data, err := active.MarshalJSON()
	if err != nil {
		return err
	}
	path := filepath.Join(state.root, "active.json")
	if err := process.WritePrivate(ctx, path, data); err != nil {
		return err
	}
	l.active = path
	return nil
}
