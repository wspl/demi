// Package runnerproc runs the real demi-runner as a child process with a home
// and an installation state of its own, registered with a backend at a URL. It
// prints its pairing codes when it waits to be paired, and keeps its device
// token in its state across restarts. The suite pairs runners with it, and the
// scripted machine manager runs a Cloud's sandbox as one.
package runnerproc

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/wspl/demi/go/backendtest/procgroup"
)

// PairingPrefix is what the runner prints before each pairing code.
const PairingPrefix = "demi-runner: pairing code: "

// pairingPatience is how long a runner may take to print a pairing code.
const pairingPatience = 15 * time.Second

// stopPatience is how long a runner asked to stop has before it is killed.
const stopPatience = 5 * time.Second

// Options say how a runner starts.
type Options struct {
	// Name is the device name the runner reports.
	Name string
	// Env is the runner's own environment, the device's: the test process's
	// when nil, else exactly these variables, as a Cloud's image gives its
	// runner. The runner's home, state and name are set on top.
	Env []string
	// Token puts a device token in the runner's state, so it connects as that
	// device instead of waiting to be paired.
	Token string
	// Managed says the runner is a managed host, as a Cloud's runner is.
	Managed bool
}

// A Process is a runner process with its home and state.
type Process struct {
	program string
	backend string
	options Options
	home    string
	state   string

	mu      sync.Mutex
	cmd     *exec.Cmd
	exited  chan struct{}
	output  strings.Builder
	codes   []string
	changed chan struct{}
}

// Start starts the runner program for the backend at backendURL, such as
// http://127.0.0.1:3271, in a new home and state under root.
func Start(program, backendURL, root string, options Options) (*Process, error) {
	home, err := os.MkdirTemp(root, "home-")
	if err != nil {
		return nil, err
	}
	// The runner reports its home by its real path.
	home, err = filepath.EvalSymlinks(home)
	if err != nil {
		return nil, err
	}
	state, err := os.MkdirTemp(root, "state-")
	if err != nil {
		return nil, err
	}
	if err := os.Mkdir(filepath.Join(state, "tmp"), 0o755); err != nil {
		return nil, err
	}
	if options.Token != "" {
		if err := writeToken(state, options.Token); err != nil {
			return nil, err
		}
	}
	p := &Process{
		program: program,
		backend: backendURL,
		options: options,
		home:    home,
		state:   state,
		changed: make(chan struct{}),
	}
	if err := p.spawn(); err != nil {
		return nil, err
	}
	return p, nil
}

// Home is the runner's home, where its Hosts start work.
func (p *Process) Home() string { return p.home }

// StateDir is the runner's installation state, which holds its device token.
func (p *Process) StateDir() string { return p.state }

// Output is what the runner printed.
func (p *Process) Output() string {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.output.String()
}

// PairingCode returns the index'th pairing code the runner printed, waiting for
// it until ctx ends or the runner has had its patience.
func (p *Process) PairingCode(ctx context.Context, index int) (string, error) {
	ctx, cancel := context.WithTimeout(ctx, pairingPatience)
	defer cancel()
	for {
		p.mu.Lock()
		if len(p.codes) > index {
			code := p.codes[index]
			p.mu.Unlock()
			return code, nil
		}
		changed := p.changed
		p.mu.Unlock()
		select {
		case <-changed:
		case <-ctx.Done():
			return "", fmt.Errorf("the runner printed no pairing code %d:\n%s", index, p.Output())
		}
	}
}

// Running reports whether the runner process still runs.
func (p *Process) Running() bool {
	p.mu.Lock()
	exited := p.exited
	p.mu.Unlock()
	if exited == nil {
		return false
	}
	select {
	case <-exited:
		return false
	default:
		return true
	}
}

// Stop asks the runner to stop and waits for it, killing it after five seconds.
func (p *Process) Stop() {
	p.end(syscall.SIGTERM, stopPatience)
}

// Kill kills the runner at once, as a crash would.
func (p *Process) Kill() {
	p.end(syscall.SIGKILL, 0)
}

// Remove kills the runner and everything it started, which a test that is over
// leaves nothing of.
func (p *Process) Remove() {
	p.mu.Lock()
	cmd := p.cmd
	p.mu.Unlock()
	if cmd != nil {
		procgroup.Kill(cmd)
	}
	p.end(syscall.SIGKILL, 0)
}

// StartAgain starts the runner again after a stop, with the same home and
// state.
func (p *Process) StartAgain() error {
	if p.Running() {
		return errors.New("the runner still runs")
	}
	return p.spawn()
}

// StartAgainWithToken starts the runner again after a stop for the backend at
// backendURL with token as its device token, as a Cloud boots with the boot
// record of that boot; its home and the rest of its state stay.
func (p *Process) StartAgainWithToken(backendURL, token string) error {
	if p.Running() {
		return errors.New("the runner still runs")
	}
	p.backend = backendURL
	if err := writeToken(p.state, token); err != nil {
		return err
	}
	return p.spawn()
}

// ClearState empties the runner's state while it is stopped, as a Cloud reset
// replaces the system the runner's state lives on; its home stays.
func (p *Process) ClearState() error {
	return p.clearStateBut()
}

// ClearRunState empties the runner's state while it is stopped as a Cloud's
// stop does: its temporary state, /run/demi on a Cloud, goes, while its log and
// its job directories, which a Cloud keeps on its system image, and its home
// stay.
func (p *Process) ClearRunState() error {
	return p.clearStateBut("log", "jobs")
}

func (p *Process) clearStateBut(kept ...string) error {
	if p.Running() {
		return errors.New("the runner still runs")
	}
	entries, err := os.ReadDir(p.state)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		keep := false
		for _, name := range kept {
			if entry.Name() == name {
				keep = true
			}
		}
		if keep {
			continue
		}
		if err := os.RemoveAll(filepath.Join(p.state, entry.Name())); err != nil {
			return err
		}
	}
	return os.Mkdir(filepath.Join(p.state, "tmp"), 0o755)
}

// Token returns the device token the runner stored, and whether it has stored
// one.
func (p *Process) Token() (string, bool) {
	data, err := os.ReadFile(filepath.Join(p.state, "runner-token"))
	if err != nil {
		return "", false
	}
	return strings.TrimSpace(string(data)), true
}

// Command returns another runner set up as this one: the same home, state and
// backend, not started.
func (p *Process) Command() *exec.Cmd {
	cmd := exec.Command(p.program, "run", "--backend", p.backend)
	if p.options.Env != nil {
		cmd.Env = append([]string(nil), p.options.Env...)
	} else {
		cmd.Env = os.Environ()
	}
	cmd.Env = withEnv(cmd.Env,
		"HOME="+p.home,
		"USERPROFILE="+p.home,
		"DEMI_HOME="+p.state,
		// What the runner leaves in its temporary directory goes with its
		// state, even when it is killed.
		"TMPDIR="+filepath.Join(p.state, "tmp"),
		"DEMI_RUNNER_NAME="+p.options.Name,
	)
	if p.options.Managed {
		cmd.Env = withEnv(cmd.Env, "DEMI_RUNNER_MANAGED=1")
	} else {
		cmd.Env = without(cmd.Env, "DEMI_RUNNER_MANAGED")
	}
	cmd.Dir = p.home
	return cmd
}

func (p *Process) spawn() error {
	cmd := p.Command()
	// The runner writes to pipes of ours rather than ones os/exec owns, so
	// that Wait returns when the runner exits, whatever a process it started
	// still holds open.
	readOutput, writeOutput, err := os.Pipe()
	if err != nil {
		return err
	}
	cmd.Stdout = writeOutput
	cmd.Stderr = writeOutput
	err = procgroup.Start(cmd)
	// The runner has its own copy of the write end.
	_ = writeOutput.Close()
	if err != nil {
		_ = readOutput.Close()
		return fmt.Errorf("start %s: %w", p.program, err)
	}
	exited := make(chan struct{})
	p.mu.Lock()
	p.cmd = cmd
	p.exited = exited
	p.mu.Unlock()
	go func() {
		// The reader ends with the last holder of the write end.
		defer readOutput.Close()
		p.read(readOutput)
	}()
	go func() {
		// The exit status of a runner that was stopped or killed is of no use.
		_ = cmd.Wait()
		close(exited)
	}()
	return nil
}

func (p *Process) read(stream io.Reader) {
	lines := bufio.NewScanner(stream)
	lines.Buffer(nil, 1<<20)
	for lines.Scan() {
		line := lines.Text()
		p.mu.Lock()
		p.output.WriteString(line)
		p.output.WriteByte('\n')
		if code, ok := strings.CutPrefix(line, PairingPrefix); ok {
			p.codes = append(p.codes, strings.TrimSpace(code))
			close(p.changed)
			p.changed = make(chan struct{})
		}
		p.mu.Unlock()
	}
}

// end signals the runner and waits for it; a runner that has not exited within
// patience of a signal that is not a kill is killed.
func (p *Process) end(signal syscall.Signal, patience time.Duration) {
	p.mu.Lock()
	cmd := p.cmd
	exited := p.exited
	p.mu.Unlock()
	if cmd == nil || cmd.Process == nil || exited == nil {
		return
	}
	select {
	case <-exited:
		return
	default:
	}
	// A runner that exited meanwhile has nothing to signal.
	_ = cmd.Process.Signal(signal)
	if patience > 0 {
		select {
		case <-exited:
			return
		case <-time.After(patience):
			_ = cmd.Process.Kill()
		}
	}
	<-exited
}

func writeToken(state, token string) error {
	return os.WriteFile(filepath.Join(state, "runner-token"), []byte(token+"\n"), 0o600)
}

// withEnv returns env with the assignments set, replacing a variable of the same
// name.
func withEnv(env []string, assignments ...string) []string {
	for _, assignment := range assignments {
		name, _, _ := strings.Cut(assignment, "=")
		env = append(without(env, name), assignment)
	}
	return env
}

func without(env []string, name string) []string {
	kept := make([]string, 0, len(env))
	for _, entry := range env {
		if entryName, _, _ := strings.Cut(entry, "="); entryName != name {
			kept = append(kept, entry)
		}
	}
	return kept
}
