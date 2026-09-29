package hostremote_test

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"io"
	"net"
	"os"
	"strings"
	"testing"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandtree"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/hostremote"
	"github.com/wspl/demi/go/hostremote/hostremotetest"
	"github.com/wspl/demi/go/shell"
	"github.com/wspl/demi/go/shell/shelltest"
)

func caller(t *testing.T) shell.JobCaller {
	t.Helper()
	node, err := core.ParseNodeID("node")
	if err != nil {
		t.Fatal(err)
	}
	return shell.JobCaller{Node: node, Generation: 1}
}
func shellOn(t *testing.T, f *hostremotetest.RunnerFixture, selection *hostremote.CommandSelection) (*hostremote.RemoteShellEnvironment, *shelltest.Pages) {
	t.Helper()
	pages := shelltest.NewPages(true)
	options := hostremote.NewEnvironmentOptions(f.Host(), f.Device.Execute, func(context.Context) (commandservice.CommandContext, error) { return shelltest.CommandContext(), nil }, pages, &shelltest.CountingNumbers{})
	options.Commands = selection
	env := hostremote.NewRemoteShellEnvironment(options)
	t.Cleanup(func() {
		if err := env.DisposeAll(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return env, pages
}
func runShell(t *testing.T, env *hostremote.RemoteShellEnvironment, script string) shell.CommandStatus {
	t.Helper()
	// Ten minutes is only a hang guard; process exit decides this result.
	window, _ := shell.NewObservationWindow(600000)
	status, err := env.Exec(t.Context(), shell.ExecRequest{Script: script, Caller: caller(t), Window: window})
	if err != nil {
		t.Fatal(err)
	}
	if status.State.Phase != shell.CommandExited {
		t.Fatalf("command did not exit: %+v", status)
	}
	return status
}
func whole(status shell.CommandStatus) string {
	if status.Whole == nil {
		return status.Output.Text
	}
	return status.Whole.Output.Text(shell.Streams{}, nil, shell.Seen{}).Display()
}

// Cost: one real runner and small files/processes. No scenario outcome depends
// on elapsed wall time; all process and stream waits are event-driven.
func TestRustRunnerHostConformanceAndShellLifecycle(t *testing.T) {
	fixture := hostremotetest.StartFixture(t, hostremotetest.FixtureOptions{Env: map[string]*string{"DEVICE_VALUE": new("inherited")}})
	shelltest.HostConformance(t, fixture.Host(), fixture.Process.Home(), os.Getenv("PATH"))
	env, pages := shellOn(t, fixture, nil)
	first := runShell(t, env, "mkdir -p child; cd child; export TRANSIENT=gone; printf '%s' \"$DEVICE_VALUE\"; printf err >&2")
	if first.State.ExitCode != 0 || !strings.Contains(whole(first), "inherited") {
		t.Fatalf("first: %+v %s", first.State, whole(first))
	}
	second := runShell(t, env, "printf '%s|%s' \"$PWD\" \"${TRANSIENT-unset}\"")
	if second.ShellID != first.ShellID || !strings.Contains(whole(second), "/child|unset") {
		t.Fatal(whole(second))
	}
	// Disposal ends the current shells; the environment can run fresh ones.
	if err := env.DisposeAll(context.Background()); err != nil {
		t.Fatal(err)
	}
	fresh := runShell(t, env, "printf '%s' \"$PWD\"")
	if fresh.ShellID == first.ShellID || string(fresh.Whole.Output.Text(shell.Streams{}, nil, shell.Seen{}).Bytes()) != fixture.Process.Home() {
		t.Fatal("environment did not reset after disposal")
	}
	big := runShell(t, env, "head -c 100000 /dev/zero | tr '\\0' x")
	if big.Whole == nil || len(big.Whole.Output.Text(shell.Streams{}, nil, shell.Seen{}).Bytes()) != 100000 {
		t.Fatalf("kept output: %d", len(whole(big)))
	}
	binary := runShell(t, env, "printf '\\000\\377A'")
	if binary.State.BinaryStdout == nil || !bytes.Equal(binary.State.BinaryStdout.Bytes, []byte{0, 255, 'A'}) {
		t.Fatalf("binary: %+v", binary.State)
	}
	pages.Drain()
	result := make(chan shell.CommandStatus, 1)
	failed := make(chan error, 1)
	window, _ := shell.NewObservationWindow(600000)
	go func() {
		status, err := env.Exec(t.Context(), shell.ExecRequest{Script: "printf ready; read line; printf '%s' \"$line\"; read rest", Caller: caller(t), Window: window})
		if err != nil {
			failed <- err
			return
		}
		result <- status
	}()
	var command core.CommandID
	for {
		page, err := pages.Next(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		if strings.Contains(page.Tail, "ready") {
			command = page.CommandID
			break
		}
	}
	if err := env.Write(t.Context(), command, []byte("input\n")); err != nil {
		t.Fatal(err)
	}
	for {
		page, err := pages.Next(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		if strings.Contains(page.Tail, "input") {
			break
		}
	}
	if err := env.Abort(t.Context(), command); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-failed:
		t.Fatal(err)
	case status := <-result:
		if status.State.Phase != shell.CommandAborted {
			t.Fatal(status.State)
		}
	}
}

// Cost: one runner and one native fixture service. This crosses the MessagePack
// runner wire, artifact admission, HTTP pipes, RPC storage and service lifecycle.
func TestRustRunnerRPCNativeServicesAndFacets(t *testing.T) {
	native := hostremotetest.LoadNativeFixture(t)
	commands := &shell.CommandSet{}
	declaration := shell.DeclareLeaf(commandtree.Leaf{Name: "relay", Summary: "relay input", Kind: commandtree.KindRPC}, shell.RPCHandlerFunc(func(ctx context.Context, _ shell.RPCInvocation, port shell.RPCPort) (uint8, error) {
		reply, err := port.Storage(ctx, shell.StorageWriteIf{Key: "seen", Value: jsontext.Value(`true`)})
		if err != nil {
			return 0, err
		}
		if _, ok := reply.(shell.StorageCommitted); !ok {
			return 0, errors.New("not committed")
		}
		for {
			data, err := port.ReadStdin(ctx)
			if err != nil {
				return 0, err
			}
			if data == nil {
				break
			}
			if err := port.Stdout(ctx, data); err != nil {
				return 0, err
			}
		}
		return 0, port.Stderr(ctx, []byte("diagnostic\n"))
	}))
	if err := commands.Register(declaration); err != nil {
		t.Fatal(err)
	}
	if err := commands.Register(shell.DeclareLeaf(commandtree.Leaf{Name: "native-echo", Kind: commandtree.KindNative, Binding: &commandtree.Binding{Package: native.Descriptor.ID, Operation: "echo"}}, nil)); err != nil {
		t.Fatal(err)
	}
	fixture := hostremotetest.StartFixture(t, hostremotetest.FixtureOptions{Commands: commands})
	catalog, err := hostremote.NewCommandCatalog([]commandservice.PackageDescriptor{native.Descriptor}, native)
	if err != nil {
		t.Fatal(err)
	}
	selected, err := catalog.Select(commands)
	if err != nil {
		t.Fatal(err)
	}
	env, _ := shellOn(t, fixture, selected)
	answer := runShell(t, env, "printf payload | relay")
	if answer.State.ExitCode != 0 || !strings.Contains(whole(answer), "payload") || !strings.Contains(whole(answer), "diagnostic") {
		t.Fatal(whole(answer))
	}
	if string(fixture.Policy.StorageFor("node").Value("seen")) != "true" {
		t.Fatal("storage missing")
	}
	nativeResult := runShell(t, env, "printf native-payload | native-echo")
	if nativeResult.State.ExitCode != 0 || string(nativeResult.Whole.Output.Text(shell.Streams{}, nil, shell.Seen{}).Bytes()) != "native-payload" {
		t.Fatal(whole(nativeResult))
	}
	user := shelltest.CommandContext()
	user.Caller = commandservice.UserCaller{}
	request := hostremote.ServiceRequest{Context: user, Package: native.Descriptor, Operation: "echo", Cwd: fixture.Process.Home(), Resolver: native}
	payload := bytes.Repeat([]byte{0, 255, 'a'}, 30000)
	echoed, err := fixture.Host().CallService(t.Context(), request, payload, 100000)
	if err != nil || !bytes.Equal(echoed, payload) {
		t.Fatalf("service echo %d: %v", len(echoed), err)
	}
	request.Operation = "where"
	request.Args = new(jsontext.Value(`{"label":"probe"}`))
	request.JSON = new(true)
	data, err := fixture.Host().CallService(t.Context(), request, nil, 10000)
	if err != nil {
		t.Fatal(err)
	}
	var location map[string]any
	if err := json.Unmarshal(data, &location); err != nil || location["cwd"] != fixture.Process.Home() || location["label"] != "probe" {
		t.Fatalf("native context: %s %v", data, err)
	}
	request.Operation = "result"
	request.Args = nil
	request.JSON = nil
	_, err = fixture.Host().CallService(t.Context(), request, nil, 10000)
	var serviceError *hostremote.ServiceCallError
	if !errors.As(err, &serviceError) || serviceError.ExitCode != 17 || len(serviceError.Stdout) == 0 {
		t.Fatalf("service exit: %v", err)
	}
	request.Operation = "echo"
	_, err = fixture.Host().CallService(t.Context(), request, []byte("too long"), 1)
	if !errors.As(err, &serviceError) || serviceError.Limit == nil {
		t.Fatalf("service bound: %v", err)
	}
	if err := fixture.Host().ReleaseConversation(t.Context(), user.Conversation); err != nil {
		t.Fatal(err)
	}
	log, err := fixture.Host().ReadLog(t.Context(), nil, 1000, nil)
	if err != nil || len(log.Lines) == 0 {
		t.Fatalf("log: %+v %v", log, err)
	}
	host := fixture.Host()
	p, err := host.Spawn(t.Context(), shell.SpawnRequest{Command: "git", Args: []string{"init", "repo"}})
	if err != nil {
		t.Fatal(err)
	}
	for {
		_, err := p.Output.Next(t.Context())
		if err == io.EOF {
			break
		}
		if err != nil {
			t.Fatal(err)
		}
	}
	if end, err := p.Wait(t.Context()); err != nil || end.ExitCode != 0 {
		t.Fatal(end, err)
	}
	p.Close()
	committed := runShell(t, env, "cd repo; printf 'before\\n' > tracked; git add tracked; git -c user.name=Test -c user.email=test@example.invalid commit -qm initial; printf 'after\\n' > tracked")
	if committed.State.ExitCode != 0 {
		t.Fatal(whole(committed))
	}
	original, err := host.GitShow(t.Context(), fixture.Process.Home()+"/repo", "tracked")
	if err != nil || string(original) != "before\n" {
		t.Fatalf("git show %q: %v", original, err)
	}
	changes, err := host.GitChanges(t.Context(), fixture.Process.Home()+"/repo")
	if err != nil || !changes.Repository || len(changes.Files) != 1 || changes.Files[0].Path != "tracked" {
		t.Fatalf("git: %+v %v", changes, err)
	}
}

// Cost: one runner, one loopback socket and 1 MiB. EOF proves half-close;
// no sleep or throughput deadline decides success.
func TestRustRunnerNetworkPipeRoundTrip(t *testing.T) {
	fixture := hostremotetest.StartFixture(t, hostremotetest.FixtureOptions{})
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	peer := make(chan error, 1)
	go func() {
		socket, err := listener.Accept()
		if err != nil {
			peer <- err
			return
		}
		defer socket.Close()
		_, err = io.Copy(socket, socket)
		peer <- err
	}()
	input, output := fixture.Device.Pipes.ToDevice(hostremotetest.TestDeviceID), fixture.Device.Pipes.FromDevice(hostremotetest.TestDeviceID)
	writer, err := input.Writer()
	if err != nil {
		t.Fatal(err)
	}
	reader, err := output.Reader()
	if err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	if err := fixture.Host().OpenNet(t.Context(), "127.0.0.1", uint16(listener.Addr().(*net.TCPAddr).Port), input.WireRef(), output.WireRef()); err != nil {
		t.Fatal(err)
	}
	payload := bytes.Repeat([]byte{0, 255, 'a', 'b'}, 262144)
	sent := make(chan error, 1)
	go func() {
		_, err := writer.WriteContext(t.Context(), payload)
		writer.Close()
		sent <- err
	}()
	result, err := io.ReadAll(reader)
	if err != nil || !bytes.Equal(result, payload) {
		t.Fatalf("echo %d: %v", len(result), err)
	}
	if err := <-sent; err != nil {
		t.Fatal(err)
	}
	if err := <-peer; err != nil {
		t.Fatal(err)
	}
}
