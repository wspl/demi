package remotehost_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerwire"
)

var runnerAvailability struct {
	once    sync.Once
	waiting bool
	err     error
}

// runnerFixture builds through programtest and skips only the explicit r-runner placeholder.
// Build failures and failures from a migrated runner remain failures.
func runnerFixture(t *testing.T, options remotehosttest.FixtureOptions) *remotehosttest.RunnerFixture {
	t.Helper()
	runnerAvailability.once.Do(func() {
		path, err := remotehosttest.RunnerBinary(t.Context())
		if err != nil {
			runnerAvailability.err = err
			return
		}
		output, err := exec.CommandContext(t.Context(), path, "--help").CombinedOutput()
		var exit *exec.ExitError
		runnerAvailability.waiting = errors.As(err, &exit) && exit.ExitCode() == 1 && string(output) == "demi-runner: not migrated yet\n"
		if err != nil && !runnerAvailability.waiting {
			runnerAvailability.err = fmt.Errorf("runner --help: %w: %s", err, output)
		}
	})
	requirePipe(t, runnerAvailability.err)
	if runnerAvailability.waiting {
		t.Skip("waiting for r-runner: programtest built the explicit migration placeholder")
	}
	f, err := remotehosttest.StartRunnerFixture(t.Context(), t, options)
	requirePipe(t, err)
	return f
}

// runnerShell owns a shell's jobs and observes their completion through page events.
func runnerShell(t *testing.T, f *remotehosttest.RunnerFixture, env map[string]string, commands *remotehost.CommandSelection) (*remotehost.ShellEnvironment, *hosttest.Pages) {
	t.Helper()
	pages := hosttest.NewPages(false)
	options := remotehost.NewEnvironmentOptions(f.Host(), func(context.Context) (commandwire.CommandContext, error) { return hosttest.CommandContext(), nil }, pages, &hosttest.CountingNumbers{})
	options.InitialEnv = map[string]string{"PATH": "/usr/bin:/bin"}
	for name, value := range env {
		options.InitialEnv[name] = value
	}
	options.Commands = commands
	shell := remotehost.NewShellEnvironment(options)
	t.Cleanup(func() {
		requirePipe(t, f.Stop(context.Background()))
		requirePipe(t, shell.DisposeAll(context.Background()))
	})
	return shell, pages
}

// runnerExec observes for the requested window; the test owns the command lifetime.
func runnerExec(t *testing.T, s *remotehost.ShellEnvironment, script string, millis uint64) host.CommandStatus {
	t.Helper()
	window, ok := host.NewObservationWindow(millis)
	if !ok {
		t.Fatal("invalid observation window")
	}
	result, err := s.Exec(t.Context(), host.ExecRequest{Script: script, Window: window, Caller: host.JobCaller{Node: "test-session"}, ToolUseID: "call"})
	requirePipe(t, err)
	return result
}

// runnerEnd consumes page events until this command has settled.
func runnerEnd(t *testing.T, s *remotehost.ShellEnvironment, p *hosttest.Pages, id core.CommandID) host.CommandStatus {
	t.Helper()
	for {
		status, err := s.Status(id)
		requirePipe(t, err)
		if status.State.Phase != host.Running {
			return status
		}
		view := page(t, p)
		if view.CommandID == id && view.State.Phase != host.Running {
			status, err = s.Status(id)
			requirePipe(t, err)
			return status
		}
	}
}

// processOutput drains output before joining the runner's terminal result.
func processOutput(t *testing.T, p *host.StartedProcess) ([]byte, host.ProcessEnd) {
	t.Helper()
	var data []byte
	for {
		chunk, err := p.Output.Next(t.Context())
		if errors.Is(err, io.EOF) {
			break
		}
		requirePipe(t, err)
		if chunk.Stream == core.StreamKindStdout {
			data = append(data, chunk.Bytes...)
		}
	}
	end, err := p.Wait(t.Context())
	requirePipe(t, err)
	return data, end
}

// patternedBytes exposes misplaced file ranges and reordered pipe chunks.
func patternedBytes(size int) []byte {
	data := make([]byte, size)
	for i := range data {
		data[i] = byte((i*31 + (i >> 8)) % 251)
	}
	return data
}

// wholeStream extracts the asserted stream without relying on inter-stream scheduling.
func wholeStream(output host.WholeOutput, stream core.StreamKind) []byte {
	var data []byte
	for _, record := range output.Records {
		if record.Stream == stream {
			data = append(data, record.Bytes...)
		}
	}
	return data
}

// wireTap observes runner events without polling process state or the filesystem.
type wireTap struct {
	input chan runnerwire.Outbound
	seen  []runnerwire.Outbound
}

func newWireTap() *wireTap { return &wireTap{input: make(chan runnerwire.Outbound, 1<<16)} }
func (tap *wireTap) find(t *testing.T, match func(runnerwire.Outbound) bool) runnerwire.Outbound {
	t.Helper()
	for _, message := range tap.seen {
		if match(message) {
			return message
		}
	}
	for {
		select {
		case message := <-tap.input:
			tap.seen = append(tap.seen, message)
			if match(message) {
				return message
			}
		case <-t.Context().Done():
			t.Fatal(t.Context().Err())
		}
	}
}
func (tap *wireTap) pipeDone(t *testing.T, id string) *runnerwire.PipeDone {
	t.Helper()
	return tap.find(t, func(message runnerwire.Outbound) bool {
		done, ok := message.(*runnerwire.PipeDone)
		return ok && done.PipeID == id
	}).(*runnerwire.PipeDone)
}

// Cost: real runner cases build once; each scenario uses process/pipe events and no wall-time polling.
func TestRunnerPassesHostConformanceOverWire(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	root := filepath.Join(f.Home(), "conformance")
	requirePipe(t, os.Mkdir(root, 0700))
	for _, scenario := range hosttest.ConformanceCases(f.HostAt(root), root, "/usr/bin:/bin") {
		t.Run(scenario.Name, func(t *testing.T) { requirePipe(t, scenario.Run(t.Context())) })
	}
	if !f.Host().Online() {
		t.Fatal("runner disconnected")
	}
}

func TestRunnerProcessReceivesEarlyInputAndUnreadInputDoesNotBlock(t *testing.T) {
	tap := newWireTap()
	f := runnerFixture(t, remotehosttest.FixtureOptions{Tap: tap.input})
	h := f.Host()
	cat, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "/bin/cat"})
	requirePipe(t, err)
	data := make([]byte, runnerwire.StdinChunkBytes+1)
	for i := range data {
		data[i] = byte(i % 256)
	}
	requirePipe(t, cat.Control.WriteStdin(t.Context(), []byte("early input|")))
	requirePipe(t, cat.Control.WriteStdin(t.Context(), data))
	requirePipe(t, cat.Control.CloseStdin(t.Context()))
	sleeper, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "/bin/sleep", Args: []string{"10"}})
	requirePipe(t, err)
	requirePipe(t, sleeper.Control.Kill(t.Context(), host.Kill))
	output, end := processOutput(t, cat)
	if !bytes.Equal(output, append([]byte("early input|"), data...)) || end.Kind != host.ProcessExited || end.ExitCode != 0 {
		t.Fatal("cat input changed", end)
	}
	_, end = processOutput(t, sleeper)
	if end.Kind != host.ProcessSignalled || end.Signal != "SIGKILL" {
		t.Fatal(end)
	}
	request := startRequest("printf ready; sleep 30")
	request.CWD = f.Home()
	job, err := h.StartJob(t.Context(), request)
	requirePipe(t, err)
	var ready []byte
	for !bytes.HasSuffix(ready, []byte("ready")) {
		chunk, err := job.NextOutput(t.Context())
		requirePipe(t, err)
		if chunk.Stream == core.StreamKindStdout {
			ready = append(ready, chunk.Bytes...)
		}
	}
	p, err := h.Process().Spawn(t.Context(), host.SpawnRequest{Command: "/bin/sh", Args: []string{"-c", "printf ready; sleep 30"}})
	requirePipe(t, err)
	ready = nil
	for !bytes.HasSuffix(ready, []byte("ready")) {
		chunk, err := p.Output.Next(t.Context())
		requirePipe(t, err)
		if chunk.Stream == core.StreamKindStdout {
			ready = append(ready, chunk.Bytes...)
		}
	}
	for range 4 {
		requirePipe(t, job.WriteStdin(t.Context(), make([]byte, runnerwire.StdinChunkBytes)))
		requirePipe(t, p.Control.WriteStdin(t.Context(), make([]byte, runnerwire.StdinChunkBytes)))
	}
	exists, err := h.FS().Exists(t.Context(), f.Home())
	requirePipe(t, err)
	if !exists {
		t.Fatal("runner stopped serving")
	}
	requirePipe(t, job.Kill(t.Context(), runnerwire.SignalKill))
	requirePipe(t, p.Control.Kill(t.Context(), host.Kill))
	_, end = processOutput(t, p)
	if end.Kind != host.ProcessSignalled || end.Signal != "SIGKILL" {
		t.Fatal(end)
	}
	_, err = job.End(t.Context())
	requirePipe(t, err)
	exit := tap.find(t, func(message runnerwire.Outbound) bool {
		exit, ok := message.(*runnerwire.JobExit)
		return ok && exit.JobID == job.ID()
	}).(*runnerwire.JobExit)
	if exit.Signal == nil || *exit.Signal != "SIGKILL" {
		t.Fatal(exit)
	}
}

func TestRunnerJobStreamsAndDeviceEnvironment(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{Env: map[string]string{"DEVICE_FACT": "from the device", "SHARED": "device"}})
	s, _ := runnerShell(t, f, nil, nil)
	result := runnerExec(t, s, "echo hello; echo oops >&2; exit 4", 10000)
	if result.State.Phase != host.Exited || result.State.ExitCode != 4 || result.Stdout.Delta != "hello\n" || result.Stderr.Delta != "oops\n" {
		t.Fatal(result)
	}
	for _, stream := range []core.StreamKind{core.StreamKindStdout, core.StreamKindStderr} {
		var text string
		for _, chunk := range result.Output.Chunks {
			if chunk.Stream == stream {
				text += chunk.Text
			}
		}
		want := "hello\n"
		if stream == core.StreamKindStderr {
			want = "oops\n"
		}
		if text != want {
			t.Fatal(text)
		}
	}
	link, err := f.Link(t.Context())
	requirePipe(t, err)
	if link.RunningJobs() != 0 {
		t.Fatal("job remained running")
	}
	result = runnerExec(t, s, `echo "$DEVICE_FACT|$SHARED|${DEMI_SESSION_ID:-none}|${DEMI_SHELL_ID:-none}|${DEMI_CONVERSATION_ID:-none}|${DEMI_AGENT_NODE_ID:-none}"; echo "$PATH"`, 10000)
	facts, path, _ := strings.Cut(result.Stdout.Delta, "\n")
	if facts != "from the device|device|none|none|none|none" || !strings.Contains(":"+strings.TrimSpace(path)+":", ":/usr/bin:") {
		t.Fatal(result.Stdout)
	}
	overriding, _ := runnerShell(t, f, map[string]string{"SHARED": "backend"}, nil)
	if runnerExec(t, overriding, `echo "$SHARED"`, 10000).Stdout.Delta != "backend\n" {
		t.Fatal("shell environment did not override device")
	}
}

func TestRunnerShellCarriesOnlyWorkingDirectory(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	requirePipe(t, os.Mkdir(filepath.Join(f.Home(), "sub"), 0700))
	s, _ := runnerShell(t, f, nil, nil)
	for _, scenario := range []struct {
		script, output string
		code           int32
	}{{"cd sub && export FOO=1 && pwd", f.Home() + "/sub\n", 0}, {`pwd; echo "${FOO:-unset}"`, f.Home() + "/sub\nunset\n", 0}, {"cd ..; exit 3", "", 3}, {"pwd", f.Home() + "\n", 0}, {"cd sub; do", "", 2}, {"pwd", f.Home() + "\n", 0}} {
		result := runnerExec(t, s, scenario.script, 10000)
		if result.State.Phase != host.Exited || result.State.ExitCode != scenario.code || result.Stdout.Delta != scenario.output {
			t.Fatal(scenario.script, result)
		}
	}
	window, _ := host.NewObservationWindow(10000)
	ephemeral, err := s.Exec(t.Context(), host.ExecRequest{Script: "pwd", Shell: host.ShellTarget{Kind: host.EphemeralShell, CWD: new(f.Home() + "/sub")}, Window: window, Caller: host.JobCaller{Node: "test-session"}, ToolUseID: "call"})
	requirePipe(t, err)
	if ephemeral.Stdout.Delta != f.Home()+"/sub\n" || runnerExec(t, s, "pwd", 10000).Stdout.Delta != f.Home()+"\n" {
		t.Fatal("ephemeral shell changed default cwd")
	}
}

func TestRunnerJobOutlivesWindowTakesInputAndAborts(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	s, p := runnerShell(t, f, nil, nil)
	running := runnerExec(t, s, "echo ready; head -n1; sleep 30", 200)
	if running.State.Phase != host.Running {
		t.Fatal(running)
	}
	for {
		status, err := s.Status(running.CommandID)
		requirePipe(t, err)
		if status.Stdout.Tail == "ready\n" {
			break
		}
		page(t, p)
	}
	link, err := f.Link(t.Context())
	requirePipe(t, err)
	if link.RunningJobs() != 1 {
		t.Fatal("job not counted")
	}
	requirePipe(t, s.Write(t.Context(), running.CommandID, []byte("typed\n")))
	written, err := s.Status(running.CommandID)
	requirePipe(t, err)
	if written.State.Phase != host.Running {
		t.Fatal(written)
	}
	requirePipe(t, s.Abort(t.Context(), running.CommandID))
	for range 2 {
		status, err := s.Status(running.CommandID)
		requirePipe(t, err)
		if status.State.Phase != host.Aborted {
			t.Fatal(status)
		}
	}
	if link.RunningJobs() != 0 {
		t.Fatal("aborted job remained running")
	}
}

func TestRunnerWholeOutputBeyondViewsAndJobCleanup(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	s, p := runnerShell(t, f, nil, nil)
	var printed strings.Builder
	for i := range 10000 {
		fmt.Fprintf(&printed, "%09d\n", i)
	}
	result := runnerExec(t, s, "seq -f '%09g' 0 9999; echo done >&2", 10000)
	if result.State.Phase != host.Exited || result.State.ExitCode != 0 || result.Whole == nil || result.Stdout.Bytes != 100000 || string(wholeStream(*result.Whole.Output, core.StreamKindStdout)) != printed.String() || string(wholeStream(*result.Whole.Output, core.StreamKindStderr)) != "done\n" {
		t.Fatal("whole output changed")
	}
	link, err := f.Link(t.Context())
	requirePipe(t, err)
	requirePipe(t, link.Sync(t.Context()))
	directories, err := f.JobDirectories(t.Context())
	requirePipe(t, err)
	if len(directories) != 0 {
		t.Fatal(directories)
	}
	running := runnerExec(t, s, "seq -f '%09g' 0 9999; echo output-ready >&2; sleep 30", 200)
	for {
		status, err := s.Status(running.CommandID)
		requirePipe(t, err)
		if strings.Contains(status.Stderr.Tail, "output-ready") {
			break
		}
		page(t, p)
	}
	kept, err := s.ReadOutput(t.Context(), running.CommandID)
	requirePipe(t, err)
	if string(wholeStream(kept, core.StreamKindStdout)) != printed.String() {
		t.Fatal("running kept output changed")
	}
	requirePipe(t, s.Abort(t.Context(), running.CommandID))
}

func TestRunnerLostConnectionEndsJobAndReconnects(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	s, p := runnerShell(t, f, nil, nil)
	running := runnerExec(t, s, `sh -c 'echo $$ > sleeper.pid; echo ready; exec sleep 30'`, 200)
	for {
		status, err := s.Status(running.CommandID)
		requirePipe(t, err)
		if strings.Contains(status.Stdout.Tail, "ready") {
			break
		}
		page(t, p)
	}
	pid, err := os.ReadFile(filepath.Join(f.Home(), "sleeper.pid"))
	requirePipe(t, err)
	link, err := f.Link(t.Context())
	requirePipe(t, err)
	link.Disconnect("the connection was lost")
	lost := runnerEnd(t, s, p, running.CommandID)
	if lost.State.Phase != host.Exited || lost.State.ExitCode != 127 || lost.Whole == nil || !bytes.Contains(wholeStream(*lost.Whole.Output, core.StreamKindStderr), []byte("the connection was lost")) {
		t.Fatal(lost)
	}
	_, err = f.Link(t.Context())
	requirePipe(t, err)
	// Reconnection completes the runner's old-link cleanup before it accepts this job.
	result := runnerExec(t, s, "kill -0 "+strings.TrimSpace(string(pid))+" 2>/dev/null && exit 1; exit 0", 10000)
	if result.State.Phase != host.Exited || result.State.ExitCode != 0 {
		t.Fatal("process outlived its connection", result)
	}
}
