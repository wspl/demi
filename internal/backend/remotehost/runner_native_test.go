package remotehost_test

import (
	"bytes"
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/backend/remotehost/testdata/fixture"
	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/types"
)

// nativeLeaf binds the declared command name to a fixture operation.
func nativeLeaf(n *remotehosttest.NativeFixture, name, operation string) commanddecl.Leaf[commanddecl.NativeOperation] {
	return commanddecl.Leaf[commanddecl.NativeOperation]{
		Name:    name,
		Summary: "The fixture's " + operation + ".",
		Kind: &commanddecl.Native[commanddecl.NativeOperation]{
			Binding: commanddecl.NativeOperation{Package: n.Descriptor.ID, Operation: operation},
		},
	}
}

// nativeCommands declares the fixture's public shell operations.
func nativeCommands(t *testing.T, n *remotehosttest.NativeFixture, name string) *host.CommandSet {
	t.Helper()
	where := nativeLeaf(n, "where", "where")
	schema, err := commanddecl.NewSchema(fixture.WhereArgsJSONSchema())
	requirePipe(t, err)
	where.Input = schema
	leaves := []host.Declared{host.Leaf(where, nil)}
	for _, operation := range []string{"echo", "spin", "result", "number"} {
		leaves = append(leaves, host.Leaf(nativeLeaf(n, operation, operation), nil))
	}
	commands := &host.CommandSet{}
	requirePipe(t, commands.Register(host.Group(name, "The native fixture.", leaves...)))
	return commands
}

// selectNative pins one fixture release to the selected commands.
func selectNative(
	t *testing.T,
	n *remotehosttest.NativeFixture,
	commands *host.CommandSet,
) *remotehost.CommandSelection {
	t.Helper()
	catalog, err := remotehost.NewCommandCatalog([]commandproto.PackageDescriptor{n.Descriptor}, n.Resolver())
	requirePipe(t, err)
	selection, err := catalog.Select(commands)
	requirePipe(t, err)
	return selection
}

// firstLine relays interactive input with the invoking job's conversation.
func firstLine(ctx context.Context, inv host.RPCInvocation, p host.RPCPort) (uint8, error) {
	var data []byte
	for {
		chunk, err := p.ReadLiveStdin(ctx)
		if err != nil {
			return 0, err
		}
		if chunk == nil {
			break
		}
		data = append(data, chunk...)
		if bytes.Contains(data, []byte{'\n'}) {
			break
		}
	}
	line, _, _ := strings.Cut(string(data), "\n")
	return 0, p.Stdout(ctx, []byte(line+"|"+inv.Context.Conversation))
}

// awaitStdout uses page publications as the command-ready event.
func awaitStdout(t *testing.T, s *remotehost.ShellEnvironment, p *hosttest.Pages, id types.CommandID, text string) {
	t.Helper()
	for {
		status, err := s.Status(id)
		requirePipe(t, err)
		if strings.Contains(status.Stdout.Tail, text) {
			return
		}
		if status.State.Phase != host.Running {
			t.Fatal("command ended before marker", status)
		}
		page(t, p)
	}
}

func TestRunnerNativeCommandUsesOwnJobContextAndRunner(t *testing.T) {
	a := runnerFixture(t, remotehosttest.FixtureOptions{})
	b := runnerFixture(t, remotehosttest.FixtureOptions{})
	native := nativeFixture(t)
	selection := selectNative(t, native, nativeCommands(t, native, "demi"))
	onA, pages := runnerShell(t, a, nil, selection)
	onB, _ := runnerShell(t, b, nil, selection)
	var fromA, fromB host.CommandStatus
	var calls sync.WaitGroup
	calls.Go(func() {
		fromA = runnerExec(
			t,
			onA,
			"DEMI_CONVERSATION_ID=forged DEMI_AGENT_NODE_ID=forged PROBE=alpha demi where --label A",
			untilExit,
		)
	})
	calls.Go(func() { fromB = runnerExec(t, onB, "PROBE=beta demi where --label B", untilExit) })
	calls.Wait()
	for _, item := range []struct {
		status            host.CommandStatus
		label, cwd, value string
	}{{fromA, "A", a.Home(), "alpha"}, {fromB, "B", b.Home(), "beta"}} {
		report, err := fixture.DecodeWhereReport([]byte(item.status.Stdout.Delta))
		requirePipe(t, err)
		if !reflect.DeepEqual(report.Context, hosttest.CommandContext()) || report.Label == nil ||
			*report.Label != item.label ||
			report.CWD != item.cwd ||
			report.Value == nil ||
			*report.Value != item.value {
			t.Fatal(report)
		}
	}
	data := make([]byte, 3*1024*1024)
	for i := range data {
		data[i] = byte(i % 256)
	}
	requirePipe(t, os.WriteFile(filepath.Join(a.Home(), "input"), data, 0o600))
	echoed := runnerExec(t, onA, "cat input | demi echo > output", untilExit)
	output, err := os.ReadFile(filepath.Join(a.Home(), "output"))
	requirePipe(t, err)
	if echoed.State.ExitCode != 0 || !bytes.Equal(output, data) {
		t.Fatal("native binary echo changed")
	}
	result := runnerExec(t, onA, "demi result", untilExit)
	if result.State.ExitCode != 17 || result.Stdout.Delta != "command output" ||
		result.Stderr.Delta != "command diagnostic" {
		t.Fatal(result)
	}
	failed := runnerExec(t, onA, "RESULT=error demi result", untilExit)
	if failed.State.ExitCode != 1 || !strings.Contains(failed.Stderr.Delta, "command failed") {
		t.Fatal(failed)
	}
	numbered := runnerExec(t, onA, "demi number && demi number", untilExit)
	if numbered.State.ExitCode != 0 || numbered.Stdout.Delta != `{"first":1}{"first":2}` {
		t.Fatal(numbered)
	}
	capture := runnerExec(
		t,
		onA,
		`printf '%s\n%s\n' "$DEMI_RUNNER_ENDPOINT" "$DEMI_CONTEXT_ID" > context; echo ready; sleep 30`,
		glance,
	)
	awaitStdout(t, onA, pages, capture.CommandID, "ready")
	captured, err := os.ReadFile(filepath.Join(a.Home(), "context"))
	requirePipe(t, err)
	endpoint, id, ok := strings.Cut(strings.TrimSpace(string(captured)), "\n")
	if !ok {
		t.Fatal(string(captured))
	}
	other := runnerExec(t, onB, `printf '%s' "$DEMI_RUNNER_ENDPOINT"`, untilExit).Stdout.Delta
	if endpoint == other {
		t.Fatal("runners shared endpoint")
	}
	binary, err := remotehosttest.RunnerBinary(t.Context())
	requirePipe(t, err)
	client := func(endpoint string) {
		t.Helper()
		command := exec.CommandContext(t.Context(), binary, "where")
		command.Args[0] = "demi"
		command.Dir = a.Home()
		command.Env = append(os.Environ(), "DEMI_RUNNER_ENDPOINT="+endpoint, "DEMI_CONTEXT_ID="+id)
		output, err := command.CombinedOutput()
		var exit *exec.ExitError
		if !errors.As(err, &exit) || exit.ExitCode() != 1 ||
			!bytes.Contains(output, []byte("not live on this runner")) {
			t.Fatalf("stale client: %v %s", err, output)
		}
	}
	client(other)
	requirePipe(t, onA.Abort(t.Context(), capture.CommandID))
	client(endpoint)
}

// nextHint observes a new hint frame then synchronizes routing before inspecting status.
func nextHint(t *testing.T, tap *wireTap, link *remotehost.Link, want *string) {
	t.Helper()
	for {
		select {
		case message := <-tap.input:
			tap.seen = append(tap.seen, message)
			hint, ok := message.(*runnerproto.JobRunningHint)
			if ok && reflect.DeepEqual(hint.Hint, want) {
				requirePipe(t, link.Sync(t.Context()))
				return
			}
		case <-t.Context().Done():
			t.Fatal(t.Context().Err())
		}
	}
}

func TestRunnerCommandShowsLeafHintUntilLeafEnds(t *testing.T) {
	// Fixture availability is checked before loading its native companion.
	base := runnerFixture(t, remotehosttest.FixtureOptions{})
	native := nativeFixture(t)
	requirePipe(t, base.Stop(t.Context()))
	nativeHint := nativeLeaf(native, "native", "first")
	nativeHint.RunningHint = new("native: do not poll")
	rpc := commanddecl.Leaf[commanddecl.NativeOperation]{
		Name:        "rpc",
		Summary:     "Wait for a line on the backend.",
		Kind:        &commanddecl.RPC[commanddecl.NativeOperation]{},
		RunningHint: new("rpc: do not poll"),
	}
	commands := &host.CommandSet{}
	requirePipe(
		t,
		commands.Register(
			host.Group(
				"attend",
				"Hint probes.",
				host.Leaf(nativeHint, nil),
				host.Leaf(nativeLeaf(native, "plain", "first"), nil),
				host.Leaf(rpc, host.RPCHandlerFunc(firstLine)),
			),
		),
	)
	tap := newWireTap()
	f := runnerFixture(t, remotehosttest.FixtureOptions{Commands: commands, Tap: tap.input})
	s, p := runnerShell(t, f, nil, selectNative(t, native, commands))
	link, err := f.Link(t.Context())
	requirePipe(t, err)
	for _, leaf := range []string{"native", "rpc"} {
		started := runnerExec(t, s, "attend "+leaf+"; sleep 30", glance)
		nextHint(t, tap, link, new(leaf+": do not poll"))
		status, err := s.Status(started.CommandID)
		requirePipe(t, err)
		if !reflect.DeepEqual(status.State.Hint, new(leaf+": do not poll")) {
			t.Fatal(status)
		}
		requirePipe(t, s.Write(t.Context(), started.CommandID, []byte("finish\n")))
		nextHint(t, tap, link, nil)
		status, err = s.Status(started.CommandID)
		requirePipe(t, err)
		if status.State.Hint != nil {
			t.Fatal(status)
		}
		requirePipe(t, s.Abort(t.Context(), started.CommandID))
		status, err = s.Status(started.CommandID)
		requirePipe(t, err)
		if status.State.Phase != host.Aborted {
			t.Fatal(status)
		}
	}
	child := runnerExec(t, s, "exec 9<&0; sh -c 'echo $$ > child.pid; exec attend native' <&9 & wait; sleep 30", glance)
	nextHint(t, tap, link, new("native: do not poll"))
	pid, err := os.ReadFile(filepath.Join(f.Home(), "child.pid"))
	requirePipe(t, err)
	kill, err := f.Host().
		Process().
		Spawn(t.Context(), host.SpawnRequest{Command: "/bin/kill", Args: []string{"-KILL", strings.TrimSpace(string(pid))}})
	requirePipe(t, err)
	_, end := processOutput(t, kill)
	if end.Kind != host.ProcessExited || end.ExitCode != 0 {
		t.Fatal(end)
	}
	nextHint(t, tap, link, nil)
	requirePipe(t, s.Abort(t.Context(), child.CommandID))
	stopped, err := s.Status(child.CommandID)
	requirePipe(t, err)
	if stopped.State.Phase != host.Aborted {
		t.Fatal(stopped)
	}
	before := len(tap.seen)
	for _, script := range []string{"attend native --help", "attend native --unknown", "attend"} {
		if runnerExec(t, s, script, untilExit).State.Phase == host.Running {
			t.Fatal(script)
		}
	}
	plain := runnerExec(t, s, "attend plain", glance)
	if plain.State.Hint != nil {
		t.Fatal(plain)
	}
	requirePipe(t, s.Write(t.Context(), plain.CommandID, []byte("done\n")))
	if runnerEnd(t, s, p, plain.CommandID).State.ExitCode != 0 {
		t.Fatal("plain leaf failed")
	}
	requirePipe(t, link.Sync(t.Context()))
	for {
		select {
		case message := <-tap.input:
			tap.seen = append(tap.seen, message)
		default:
			goto drained
		}
	}
drained:
	for _, message := range tap.seen[before:] {
		if hint, ok := message.(*runnerproto.JobRunningHint); ok && hint.Hint != nil {
			t.Fatal("hint from help, error, group, or plain leaf", hint)
		}
	}
}

func TestRunnerBusyNativeDoesNotBlockAbortAndSecondRunnerRefused(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	native := nativeFixture(t)
	s, _ := runnerShell(t, f, nil, selectNative(t, native, nativeCommands(t, native, "demi")))
	spinning := runnerExec(t, s, "demi spin", glance)
	if spinning.State.Phase != host.Running {
		t.Fatal(spinning)
	}
	exists, err := f.Host().FS().Exists(t.Context(), f.Home())
	requirePipe(t, err)
	if !exists {
		t.Fatal("runner stopped serving")
	}
	output, err := f.Command(t.Context()).CombinedOutput()
	var exit *exec.ExitError
	if !errors.As(err, &exit) || exit.ExitCode() != 1 || !bytes.Contains(output, []byte("already active")) {
		t.Fatalf("second runner: %v %s", err, output)
	}
	requirePipe(t, s.Abort(t.Context(), spinning.CommandID))
	stopped, err := s.Status(spinning.CommandID)
	requirePipe(t, err)
	if stopped.State.Phase != host.Aborted || runnerExec(t, s, "demi --help", untilExit).State.ExitCode != 0 {
		t.Fatal(stopped)
	}
}

func TestRunnerRunningJobKeepsManifestWhileNextInstallsAnother(t *testing.T) {
	f := runnerFixture(t, remotehosttest.FixtureOptions{})
	native := nativeFixture(t)
	old, p := runnerShell(t, f, nil, selectNative(t, native, nativeCommands(t, native, "demi")))
	next, _ := runnerShell(t, f, nil, selectNative(t, native, nativeCommands(t, native, "replacement")))
	// The old job blocks reading live stdin, so it still runs while the next job installs its manifest.
	started := runnerExec(t, old, "echo ready; read proceed; PROBE=old demi where > result", glance)
	if started.State.Phase != host.Running {
		t.Fatal(started)
	}
	awaitStdout(t, old, p, started.CommandID, "ready")
	result := runnerExec(t, next, "PROBE=new replacement where", untilExit)
	report, err := fixture.DecodeWhereReport([]byte(result.Stdout.Delta))
	requirePipe(t, err)
	if report.Value == nil || *report.Value != "new" {
		t.Fatal(report)
	}
	requirePipe(t, old.Write(t.Context(), started.CommandID, []byte("proceed\n")))
	finished := runnerEnd(t, old, p, started.CommandID)
	if finished.State.ExitCode != 0 {
		t.Fatal(finished)
	}
	data, err := os.ReadFile(filepath.Join(f.Home(), "result"))
	requirePipe(t, err)
	report, err = fixture.DecodeWhereReport(data)
	requirePipe(t, err)
	if report.Value == nil || *report.Value != "old" {
		t.Fatal(report)
	}
}
