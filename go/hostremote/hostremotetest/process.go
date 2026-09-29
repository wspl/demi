// Package hostremotetest supplies real and fake runners for backend contracts.
package hostremotetest

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/shell"
)

const PairingCodePrefix = "demi-runner: pairing code: "

// Program skips a scenario when the explicit native-program directory is not
// set. A configured but missing binary is a setup failure, not a skipped test.
func Program(t testing.TB, name string) string {
	t.Helper()
	directory := os.Getenv("DEMI_TEST_PROGRAMS")
	if directory == "" {
		t.Skip("DEMI_TEST_PROGRAMS is not set")
	}
	if runtime.GOOS == "windows" {
		name += ".exe"
	}
	binary := filepath.Join(directory, name)
	if _, err := os.Stat(binary); err != nil {
		t.Fatal(err)
	}
	return binary
}

type RunnerProcessOptions struct {
	Name    string
	Env     shell.SpawnEnv
	Token   *string
	Managed bool
}

// RunnerProcess owns its child and output readers. Stop/Kill join all of them;
// test cleanup does the same if a scenario fails early. Restart reuses its state.
type RunnerProcess struct {
	binary, home, state, backend string
	options                      RunnerProcessOptions
	child                        *exec.Cmd
	exited                       chan struct{}
	readers                      sync.WaitGroup
	mu                           sync.Mutex
	output                       strings.Builder
	codes                        []string
	changed                      chan struct{}
}

func StartRunner(t testing.TB, backend string, options RunnerProcessOptions) *RunnerProcess {
	t.Helper()
	home, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	process := &RunnerProcess{binary: Program(t, "demi-runner"), home: home, state: t.TempDir(), backend: backend, options: options, changed: make(chan struct{})}
	if override := os.Getenv("DEMI_RUNNER_TEST_BINARY"); override != "" {
		process.binary = override
	}
	if options.Name == "" {
		process.options.Name = "fixture"
	}
	if err := os.Mkdir(filepath.Join(process.state, "tmp"), 0700); err != nil {
		t.Fatal(err)
	}
	if options.Token != nil {
		if err := process.writeToken(*options.Token); err != nil {
			t.Fatal(err)
		}
	}
	if err := process.StartAgain(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := process.Kill(); err != nil {
			t.Error(err)
		}
	})
	return process
}
func (p *RunnerProcess) Home() string     { return p.home }
func (p *RunnerProcess) StateDir() string { return p.state }
func (p *RunnerProcess) Output() string {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.output.String()
}
func (p *RunnerProcess) Command() *exec.Cmd {
	cmd := exec.Command(p.binary, "run", "--backend", p.backend)
	cmd.Dir = p.home
	env := map[string]string{}
	if p.options.Env.Values == nil || p.options.Env.Inherit {
		for _, entry := range os.Environ() {
			name, value, ok := strings.Cut(entry, "=")
			if ok {
				env[name] = value
			}
		}
	}
	for name, value := range p.options.Env.Values {
		if value == nil {
			delete(env, name)
		} else {
			env[name] = *value
		}
	}
	env["HOME"] = p.home
	env["USERPROFILE"] = p.home
	env["DEMI_HOME"] = p.state
	env["TMPDIR"] = filepath.Join(p.state, "tmp")
	env["DEMI_RUNNER_NAME"] = p.options.Name
	if p.options.Managed {
		env["DEMI_RUNNER_MANAGED"] = "1"
	} else {
		delete(env, "DEMI_RUNNER_MANAGED")
	}
	for name, value := range env {
		cmd.Env = append(cmd.Env, name+"="+value)
	}
	slices.Sort(cmd.Env)
	return cmd
}
func (p *RunnerProcess) StartAgain() error {
	if p.child != nil {
		return errors.New("the runner still runs")
	}
	cmd := p.Command()
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		stdout.Close()
		return err
	}
	if err := cmd.Start(); err != nil {
		stdout.Close()
		stderr.Close()
		return err
	}
	p.child = cmd
	p.exited = make(chan struct{})
	p.readers.Add(2)
	for _, output := range []io.ReadCloser{stdout, stderr} {
		go func() {
			defer p.readers.Done()
			defer output.Close()
			scanner := bufio.NewScanner(output)
			scanner.Buffer(make([]byte, 4096), 1024*1024)
			for scanner.Scan() {
				line := scanner.Text()
				p.mu.Lock()
				p.output.WriteString(line)
				p.output.WriteByte('\n')
				if code, ok := strings.CutPrefix(line, PairingCodePrefix); ok {
					p.codes = append(p.codes, strings.TrimSpace(code))
					close(p.changed)
					p.changed = make(chan struct{})
				}
				p.mu.Unlock()
			}
			if err := scanner.Err(); err != nil {
				p.mu.Lock()
				fmt.Fprintf(&p.output, "output reader: %s\n", err)
				p.mu.Unlock()
			}
		}()
	}
	go func() {
		// Read both pipes to EOF before Wait closes their descriptors.
		p.readers.Wait()
		// A killed or deliberately crashing fixture may exit nonzero. Reaping it
		// is mandatory; its protocol and captured log judge the scenario.
		_ = cmd.Wait()
		close(p.exited)
	}()
	return nil
}
func (p *RunnerProcess) PairingCode(ctx context.Context, index int) (string, error) {
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
		case <-ctx.Done():
			return "", ctx.Err()
		case <-p.exited:
			return "", fmt.Errorf("the runner printed no pairing code %d:\n%s", index, p.Output())
		case <-changed:
		}
	}
}
func (p *RunnerProcess) Running() bool {
	if p.child == nil {
		return false
	}
	select {
	case <-p.exited:
		return false
	default:
		return true
	}
}
func (p *RunnerProcess) Stop() error {
	if p.child == nil {
		return nil
	}
	err := terminate(p.child.Process)
	if err != nil && !errors.Is(err, os.ErrProcessDone) {
		return err
	}
	// This is teardown's kill escalation, not a scenario's success criterion.
	timer := time.NewTimer(5 * time.Second)
	defer timer.Stop()
	select {
	case <-p.exited:
	case <-timer.C:
		return p.Kill()
	}
	p.child = nil
	return nil
}
func (p *RunnerProcess) Kill() error {
	if p.child == nil {
		return nil
	}
	err := p.child.Process.Kill()
	if err != nil && !errors.Is(err, os.ErrProcessDone) {
		return err
	}
	<-p.exited
	p.child = nil
	return nil
}
func (p *RunnerProcess) writeToken(token string) error {
	return os.WriteFile(filepath.Join(p.state, "runner-token"), []byte(token+"\n"), 0600)
}
func (p *RunnerProcess) StartAgainWithToken(backend, token string) error {
	if p.child != nil {
		return errors.New("the runner still runs")
	}
	p.backend = backend
	if err := p.writeToken(token); err != nil {
		return err
	}
	return p.StartAgain()
}
func (p *RunnerProcess) ClearState(keepPersistent bool) error {
	if p.child != nil {
		return errors.New("the runner still runs")
	}
	entries, err := os.ReadDir(p.state)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if keepPersistent && (entry.Name() == "log" || entry.Name() == "jobs") {
			continue
		}
		if err := os.RemoveAll(filepath.Join(p.state, entry.Name())); err != nil {
			return err
		}
	}
	return os.Mkdir(filepath.Join(p.state, "tmp"), 0700)
}
