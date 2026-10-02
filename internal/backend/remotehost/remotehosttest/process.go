package remotehosttest

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

import (
	"context"
	"os/exec"
	"testing"

	"github.com/wspl/demi/internal/host"
)

// PairingCode is the runner's pairing-code log prefix.
const PairingCode = "demi-runner: pairing code: "

// RunnerBinary resolves the runner through internal/programtest.
func RunnerBinary(ctx context.Context) (string, error) { panic("not written: b-remotehost") }

// NativeFixtureBinary resolves the native fixture through internal/programtest.
func NativeFixtureBinary(ctx context.Context) (string, error) { panic("not written: b-remotehost") }

// RunnerProcessOptions controls the runner's device name, environment and registration.
type RunnerProcessOptions struct {
	Name    string
	Env     host.SpawnEnv
	Token   *string
	Managed bool
}

// DefaultRunnerProcessOptions uses the fixture device name and inherited environment.
func DefaultRunnerProcessOptions() RunnerProcessOptions { panic("not written: b-remotehost") }

// RunnerProcess owns a runner, private home and state, and joined output readers.
// Its constructor registers test cleanup; Stop also permits a subsequent restart.
type RunnerProcess struct{ _ byte }

// StartRunnerProcess starts a runner for backend and registers cleanup with t.
func StartRunnerProcess(ctx context.Context, t testing.TB, backend string, options RunnerProcessOptions) (*RunnerProcess, error) {
	panic("not written: b-remotehost")
}

// Home returns the runner's home path.
func (p *RunnerProcess) Home() string { panic("not written: b-remotehost") }

// StateDir returns the runner's private installation-state path.
func (p *RunnerProcess) StateDir() string { panic("not written: b-remotehost") }

// Command prepares a runner CLI command with this process's home and state.
// The caller owns starting and waiting for the returned command.
func (p *RunnerProcess) Command(ctx context.Context) *exec.Cmd { panic("not written: b-remotehost") }

// Output returns the captured process log.
func (p *RunnerProcess) Output() string { panic("not written: b-remotehost") }

// PairingCode waits for the zero-based indexed pairing code.
func (p *RunnerProcess) PairingCode(ctx context.Context, index int) (string, error) {
	panic("not written: b-remotehost")
}

// Running reports whether the child is still running.
func (p *RunnerProcess) Running() bool { panic("not written: b-remotehost") }

// Stop stops the child gracefully and joins it and its output readers.
func (p *RunnerProcess) Stop(ctx context.Context) error { panic("not written: b-remotehost") }

// Kill kills the child and joins it and its output readers.
func (p *RunnerProcess) Kill(ctx context.Context) error { panic("not written: b-remotehost") }

// StartAgain restarts the stopped runner with its existing state.
func (p *RunnerProcess) StartAgain(ctx context.Context) error { panic("not written: b-remotehost") }

// StartAgainWithToken restarts against backend with token.
func (p *RunnerProcess) StartAgainWithToken(ctx context.Context, backend, token string) error {
	panic("not written: b-remotehost")
}

// ClearState removes the stopped runner's installation state.
func (p *RunnerProcess) ClearState(ctx context.Context) error { panic("not written: b-remotehost") }

// ClearRunState removes the stopped runner's transient run state.
func (p *RunnerProcess) ClearRunState(ctx context.Context) error { panic("not written: b-remotehost") }
