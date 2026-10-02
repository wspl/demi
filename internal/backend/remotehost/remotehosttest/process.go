package remotehosttest

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/programtest"
)

// PairingCode is the runner's pairing-code log prefix.
const PairingCode = "demi-runner: pairing code: "

// RunnerBinary resolves the runner through internal/programtest.
func RunnerBinary(ctx context.Context) (string, error) {
	return programtest.Path(ctx, "demi-runner")
}

// NativeFixtureBinary resolves the native fixture through internal/programtest.
func NativeFixtureBinary(ctx context.Context) (string, error) {
	return programtest.Path(ctx, "demi-native-fixture")
}

// RunnerProcessOptions controls the runner's device name, environment and registration.
type RunnerProcessOptions struct {
	Name    string
	Env     host.SpawnEnv
	Token   *string
	Managed bool
}

// DefaultRunnerProcessOptions uses the fixture device name and inherited environment.
func DefaultRunnerProcessOptions() RunnerProcessOptions {
	return RunnerProcessOptions{Name: "fixture", Env: host.SpawnEnv{Mode: host.Inherit}}
}

// RunnerProcess owns a runner, private home and state, and joined output readers.
// Its constructor registers test cleanup; Stop also permits a subsequent restart.
type RunnerProcess struct {
	binary, home, state, backend string
	options                      RunnerProcessOptions
	command                      *exec.Cmd
	done                         chan struct{}
	waitErr                      error
	mu                           sync.Mutex // Protects captured output, partial lines and pairing-code notifications.
	output                       strings.Builder
	codes                        []string
	changed                      chan struct{}
}

// StartRunnerProcess starts a runner for backend and registers cleanup with t.
func StartRunnerProcess(ctx context.Context, t testing.TB, backend string, options RunnerProcessOptions) (*RunnerProcess, error) {
	binary, err := RunnerBinary(ctx)
	if err != nil {
		return nil, err
	}
	home, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		return nil, err
	}
	p := &RunnerProcess{binary: binary, home: home, state: t.TempDir(), backend: backend, options: options, changed: make(chan struct{})}
	t.Cleanup(func() {
		if err := p.Stop(context.Background()); err != nil {
			t.Error(err)
		}
	})
	if err := os.Mkdir(filepath.Join(p.state, "tmp"), 0700); err != nil {
		return nil, err
	}
	if options.Token != nil {
		if err := p.writeToken(*options.Token); err != nil {
			return nil, err
		}
	}
	if err := p.StartAgain(ctx); err != nil {
		return nil, err
	}
	return p, nil
}

// Home returns the runner's home path.
func (p *RunnerProcess) Home() string {
	return p.home
}

// StateDir returns the runner's private installation-state path.
func (p *RunnerProcess) StateDir() string {
	return p.state
}

// Command prepares a runner CLI command with this process's home and state.
// The caller owns starting and waiting for the returned command.
func (p *RunnerProcess) Command(ctx context.Context) *exec.Cmd {
	command := exec.CommandContext(ctx, p.binary, "run", "--backend", p.backend)
	environment := make(map[string]string)
	if p.options.Env.Mode != host.Exactly {
		for _, entry := range os.Environ() {
			key, value, ok := strings.Cut(entry, "=")
			if ok {
				environment[key] = value
			}
		}
	}
	for key, value := range p.options.Env.Values {
		if value == nil {
			delete(environment, key)
		} else {
			environment[key] = *value
		}
	}
	environment["HOME"] = p.home
	environment["USERPROFILE"] = p.home
	environment["DEMI_HOME"] = p.state
	environment["TMPDIR"] = filepath.Join(p.state, "tmp")
	environment["DEMI_RUNNER_NAME"] = p.options.Name
	if p.options.Managed {
		environment["DEMI_RUNNER_MANAGED"] = "1"
	} else {
		delete(environment, "DEMI_RUNNER_MANAGED")
	}
	for key, value := range environment {
		command.Env = append(command.Env, key+"="+value)
	}
	command.Dir = p.home
	return command
}

// Output returns the captured process log.
func (p *RunnerProcess) Output() string {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.output.String()
}

// PairingCode waits for the zero-based indexed pairing code.
func (p *RunnerProcess) PairingCode(ctx context.Context, index int) (string, error) {
	waitCtx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	for {
		p.mu.Lock()
		if index >= 0 && index < len(p.codes) {
			code := p.codes[index]
			p.mu.Unlock()
			return code, nil
		}
		changed := p.changed
		p.mu.Unlock()
		select {
		case <-changed:
		case <-waitCtx.Done():
			return "", fmt.Errorf("the runner printed no pairing code %d:\n%s: %w", index, p.Output(), waitCtx.Err())
		}
	}
}

// Running reports whether the child is still running.
func (p *RunnerProcess) Running() bool {
	if p.command == nil {
		return false
	}
	select {
	case <-p.done:
		return false
	default:
		return true
	}
}

// Stop stops the child gracefully and joins it and its output readers.
func (p *RunnerProcess) Stop(ctx context.Context) error {
	return p.stop(ctx, false)
}

// Kill kills the child and joins it and its output readers.
func (p *RunnerProcess) Kill(ctx context.Context) error {
	return p.stop(ctx, true)
}

// StartAgain restarts the stopped runner with its existing state.
func (p *RunnerProcess) StartAgain(ctx context.Context) error {
	if p.command != nil {
		return errors.New("the runner still runs")
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	// The fixture owns the process beyond the startup caller's wait.
	command := p.Command(context.Background())
	stdout := &runnerLog{process: p}
	stderr := &runnerLog{process: p}
	command.Stdout = stdout
	command.Stderr = stderr
	if err := command.Start(); err != nil {
		return fmt.Errorf("start %s: %w", p.binary, err)
	}
	p.command = command
	p.done = make(chan struct{})
	go func() {
		p.waitErr = command.Wait()
		stdout.flush()
		stderr.flush()
		close(p.done)
	}()
	return nil
}

// StartAgainWithToken restarts against backend with token.
func (p *RunnerProcess) StartAgainWithToken(ctx context.Context, backend, token string) error {
	if p.command != nil {
		return errors.New("the runner still runs")
	}
	p.backend = backend
	if err := p.writeToken(token); err != nil {
		return err
	}
	return p.StartAgain(ctx)
}

// ClearState removes the stopped runner's installation state.
func (p *RunnerProcess) ClearState(ctx context.Context) error {
	return p.clearState(ctx, false)
}

// ClearRunState removes the stopped runner's transient run state.
func (p *RunnerProcess) ClearRunState(ctx context.Context) error {
	return p.clearState(ctx, true)
}

// Write captures the runner log and announces complete pairing-code lines.
type runnerLog struct {
	process *RunnerProcess
	partial string
}

func (w *runnerLog) Write(data []byte) (int, error) {
	w.partial += string(data)
	for {
		line, rest, ok := strings.Cut(w.partial, "\n")
		if !ok {
			break
		}
		w.partial = rest
		w.process.captureLine(strings.TrimSuffix(line, "\r"))
	}
	return len(data), nil
}

// flush publishes a final unterminated line after exec has joined the log copier.
func (w *runnerLog) flush() {
	if w.partial != "" {
		w.process.captureLine(w.partial)
		w.partial = ""
	}
}

// captureLine publishes complete runner log lines and pairing codes atomically.
func (p *RunnerProcess) captureLine(line string) {
	p.mu.Lock()
	// strings.Builder writes cannot return an error.
	p.output.WriteString(line)
	p.output.WriteByte('\n')
	if code, ok := strings.CutPrefix(line, PairingCode); ok {
		p.codes = append(p.codes, strings.TrimSpace(code))
	}
	previous := p.changed
	p.changed = make(chan struct{})
	p.mu.Unlock()
	close(previous)
}

// writeToken publishes the fixture's private device credential.
func (p *RunnerProcess) writeToken(token string) error {
	return os.WriteFile(filepath.Join(p.state, "runner-token"), []byte(token+"\n"), 0600)
}

// stop owns termination and reaping, even if its caller has canceled.
func (p *RunnerProcess) stop(ctx context.Context, kill bool) error {
	if p.command == nil {
		return nil
	}
	command := p.command
	var signalErr error
	if kill {
		signalErr = command.Process.Kill()
	} else if runtime.GOOS != "windows" {
		signalErr = command.Process.Signal(syscall.Signal(15))
	}
	if errors.Is(signalErr, os.ErrProcessDone) {
		signalErr = nil
	}
	timer := time.NewTimer(5 * time.Second)
	defer timer.Stop()
	select {
	case <-p.done:
	case <-ctx.Done():
		if err := command.Process.Kill(); err != nil && !errors.Is(err, os.ErrProcessDone) {
			signalErr = errors.Join(signalErr, err)
		}
		<-p.done
	case <-timer.C:
		if err := command.Process.Kill(); err != nil && !errors.Is(err, os.ErrProcessDone) {
			signalErr = errors.Join(signalErr, err)
		}
		<-p.done
	}
	p.command = nil
	var exit *exec.ExitError
	// A fixture intentionally stopped or killed may exit nonzero; it is still reaped.
	if p.waitErr != nil && !errors.As(p.waitErr, &exit) {
		return errors.Join(signalErr, p.waitErr)
	}
	return signalErr
}

// clearState models reset or stop by removing only the runner state that goes away.
func (p *RunnerProcess) clearState(ctx context.Context, keepPersistent bool) error {
	if p.command != nil {
		return errors.New("the runner still runs")
	}
	entries, err := os.ReadDir(p.state)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if err := ctx.Err(); err != nil {
			return err
		}
		if keepPersistent && (entry.Name() == "jobs" || entry.Name() == "log") {
			continue
		}
		if err := os.RemoveAll(filepath.Join(p.state, entry.Name())); err != nil {
			return err
		}
	}
	return os.Mkdir(filepath.Join(p.state, "tmp"), 0700)
}
