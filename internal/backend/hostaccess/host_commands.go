package hostaccess

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"strings"
	"sync"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/webapiproto"
)

// HostGroup declares demi host list, current and shell with their help text.
// Its handlers fail after the shard's Conversations owner has closed.
func HostGroup(shard HostShard) host.Declared {
	// These schemas are generated from the declarations beside this file.
	empty, _ := commanddecl.NewSchema(noArgsJSONSchema())
	shell, _ := commanddecl.NewSchema(shellArgsJSONSchema())
	return host.Group(
		"host",
		hostSummary,
		host.Leaf(
			commanddecl.Leaf[commanddecl.NativeOperation]{
				Name:    "list",
				Summary: listSummary,
				Input:   empty,
				Kind:    &commanddecl.RPC[commanddecl.NativeOperation]{},
			},
			commandVerb(
				shard,
				decodeNoArgs,
				func(ctx context.Context, call host.Call[noArgs], port host.RPCPort) (uint8, error) {
					return listHosts(ctx, shard, call.Invocation, port)
				},
			),
		),
		host.Leaf(
			commanddecl.Leaf[commanddecl.NativeOperation]{
				Name:    "current",
				Summary: currentSummary,
				Input:   empty,
				Kind:    &commanddecl.RPC[commanddecl.NativeOperation]{},
			},
			commandVerb(
				shard,
				decodeNoArgs,
				func(ctx context.Context, call host.Call[noArgs], port host.RPCPort) (uint8, error) {
					return currentHost(ctx, shard, call.Invocation, port)
				},
			),
		),
		host.Leaf(
			commanddecl.Leaf[commanddecl.NativeOperation]{
				Name:        "shell",
				Summary:     shellSummary,
				Input:       shell,
				Positionals: new([]string{"script"}),
				FailureOutput: new(
					"writes the reason to stderr and exits non-zero (127 when the host cannot run bash)",
				),
				Kind: &commanddecl.RPC[commanddecl.NativeOperation]{},
			},
			commandVerb(
				shard,
				decodeShellArgs,
				func(ctx context.Context, call host.Call[shellArgs], port host.RPCPort) (uint8, error) {
					return shellHost(ctx, shard, call, port)
				},
			),
		),
	)
}

// InvocationConversation is the conversation the invoking job belongs to.
func InvocationConversation(invocation host.RPCInvocation) (webapiproto.ConversationID, error) {
	id, err := webapiproto.ParseConversationID(invocation.Context.Conversation)
	if err != nil {
		return "", &host.RPCError{Kind: host.HandlerFailed, Message: "this session has no conversation", Err: err}
	}
	return id, nil
}

// Reachable returns the calling conversation's Hosts, with RPC errors.
func Reachable(ctx context.Context, shard HostShard, id webapiproto.ConversationID) ([]ReachableHost, error) {
	hosts, err := ConversationHosts(ctx, shard, id)
	if err != nil {
		return nil, &host.RPCError{Kind: host.HandlerFailed, Message: err.Error(), Err: err}
	}
	return hosts, nil
}

const (
	hostSummary = "The hosts this conversation reaches: list them, show the main one, run a command on another."
	listSummary = "Hosts this conversation can reach with `demi host shell --host`: " +
		"name, id, online, the directory shells start in; the main one marked."
	currentSummary = "The main host: where shell_exec runs."
	shellSummary   = "Run a shell string in another host's bash: `demi host shell --host <name|id> <script>`. " +
		"The script starts where the last shell on that host ended (its home before one ran) " +
		"with this command's stdin and stdout, byte-faithfully and streaming, " +
		"so archives pipe cleanly both ways (`demi host shell --host ci \"tar c -C /work .\" | tar x`, " +
		"`tar c . | demi host shell --host ci \"tar x -C /work\"`). stderr and the exit code pass through."
)

// listHosts renders the main-first reachable bindings with their connection state.
func listHosts(ctx context.Context, shard HostShard, invocation host.RPCInvocation, port host.RPCPort) (uint8, error) {
	id, err := InvocationConversation(invocation)
	if err != nil {
		return 0, err
	}
	hosts, err := Reachable(ctx, shard, id)
	if err != nil {
		return 0, err
	}
	if len(hosts) == 0 {
		return 0, port.Stdout(ctx, []byte("Cloud has not been allocated\n"))
	}
	var text strings.Builder
	for _, bound := range hosts {
		path := bound.Path
		if path == "" {
			path = "?"
		}
		role := "main"
		if bound.Role == Attached {
			role = "attached"
		}
		fmt.Fprintf(
			&text,
			"%s  %s  %s  %s  (%s)\n",
			bound.Name,
			bound.Device,
			connectionState(shard, bound.Device),
			path,
			role,
		)
	}
	return 0, port.Stdout(ctx, []byte(text.String()))
}

// connectionState is the product's connection label for a bound device.
func connectionState(shard HostShard, id webapiproto.DeviceID) string {
	if shard.Devices().Online(id) {
		return "online"
	}
	return "offline"
}

// currentHost renders the selected target without waking or allocating it.
func currentHost(
	ctx context.Context,
	shard HostShard,
	invocation host.RPCInvocation,
	port host.RPCPort,
) (uint8, error) {
	id, err := InvocationConversation(invocation)
	if err != nil {
		return 0, err
	}
	record, err := OwnedConversation(ctx, shard, id)
	if err != nil {
		return 0, err
	}
	target, err := ResolveTarget(ctx, shard, record)
	if err != nil {
		return 0, err
	}
	device, ok := database.ExecutionDeviceID(target)
	path := database.ExecutionPath(target)
	if !ok {
		return 0, port.Stdout(ctx, []byte(fmt.Sprintf("host: Cloud (not allocated), directory %s\n", path)))
	}
	name := string(device)
	found, ok, err := shard.Control().Device(ctx, device)
	if err != nil {
		return 0, err
	}
	if ok {
		name = found.Name
	}
	var line string
	switch selected := target.(type) {
	case *database.ExecutionWorkspace:
		workspace := string(selected.WorkspaceID)
		found, ok, err := shard.Control().Workspace(ctx, selected.WorkspaceID)
		if err != nil {
			return 0, err
		}
		if ok {
			workspace = found.Name
		}
		line = fmt.Sprintf(
			"host: workspace \"%s\" — %s on device \"%s\" (%s)\n",
			workspace,
			path,
			name,
			connectionState(shard, device),
		)
	case *database.ExecutionCloud, *database.ExecutionDevice:
		if path == "" {
			path = "home"
		}
		line = fmt.Sprintf("host: machine \"%s\" (%s, %s) — %s\n", name, device, connectionState(shard, device), path)
	}
	return 0, port.Stdout(ctx, []byte(line))
}

// shellHost resolves aliases before device IDs and preserves the command's exit text.
func shellHost(ctx context.Context, shard HostShard, call host.Call[shellArgs], port host.RPCPort) (uint8, error) {
	if strings.TrimSpace(call.Args.Script) == "" {
		return 2, port.Stderr(ctx, []byte("usage: demi host shell --host <name|id> <script>\n"))
	}
	id, err := InvocationConversation(call.Invocation)
	if err != nil {
		return 0, err
	}
	hosts, err := Reachable(ctx, shard, id)
	if err != nil {
		return 0, err
	}
	var selected *ReachableHost
	for i := range hosts {
		if hosts[i].Name == call.Args.Host {
			selected = &hosts[i]
			break
		}
	}
	if selected == nil {
		for i := range hosts {
			if string(hosts[i].Device) == call.Args.Host {
				selected = &hosts[i]
				break
			}
		}
	}
	if selected == nil {
		return 1, port.Stderr(
			ctx,
			[]byte(
				fmt.Sprintf(
					"host shell: host %s is not reachable from this conversation (see `demi host list`)\n",
					call.Args.Host,
				),
			),
		)
	}
	code, err := runOnHost(ctx, shard, id, *selected, call.Args.Script, call.Invocation, port)
	if err != nil {
		return 1, port.Stderr(ctx, []byte("host shell: "+err.Error()+"\n"))
	}
	return code, nil
}

// runOnHost carries the invoking command's identity and pipes across one admitted job.
func runOnHost(
	ctx context.Context,
	shard HostShard,
	id webapiproto.ConversationID,
	target ReachableHost,
	script string,
	invocation host.RPCInvocation,
	port host.RPCPort,
) (uint8, error) {
	stdin, stdout, err := bindHostPipes(shard, target, invocation)
	if err != nil {
		return 0, err
	}
	ended, err := WithHost(
		ctx,
		shard,
		id,
		&target.Device,
		func(ctx context.Context, admitted *ConversationHost) (remotehost.JobEnd, error) {
			job := hostJob{
				shard:      shard,
				id:         id,
				target:     target,
				script:     script,
				invocation: invocation,
				port:       port,
				stdin:      stdin,
				stdout:     stdout,
			}
			return job.run(ctx, admitted)
		},
	)
	if errors.Is(err, errNotStarted) {
		return 130, nil
	}
	if err != nil {
		return 0, err
	}
	if target.Role == Attached && ended.CWD != nil {
		if err := shard.Control().
			SetAttachedCWD(context.WithoutCancel(ctx), id, target.Device, *ended.CWD); err != nil {
			return 0, err
		}
	}
	return hostJobStatus(ctx, port, ended.Status)
}

// commandVerb owns each product host command until its IO and child jobs end.
func commandVerb[A any](
	shard HostShard,
	decode func([]byte) (A, error),
	run func(context.Context, host.Call[A], host.RPCPort) (uint8, error),
) host.RPCHandler {
	return host.TypedRPC(decode, func(ctx context.Context, call host.Call[A], port host.RPCPort) (uint8, error) {
		done, err := shard.Conversations().begin()
		if err != nil {
			return 0, &host.RPCError{Kind: host.HandlerFailed, Message: "the backend is shutting down", Err: err}
		}
		defer done()
		lifetime, cancel := context.WithCancel(ctx)
		cancelled := make(chan struct{})
		stop := context.AfterFunc(shard.Conversations().ctx, func() {
			defer close(cancelled)
			cancel()
		})
		defer func() {
			if !stop() {
				<-cancelled
			}
			cancel()
		}()
		return run(lifetime, call, port)
	})
}

type hostJob struct {
	shard         HostShard
	id            webapiproto.ConversationID
	target        ReachableHost
	script        string
	invocation    host.RPCInvocation
	port          host.RPCPort
	stdin, stdout *remotehost.Pipe
}

var errNotStarted = errors.New("the command ended before it started")

func (j hostJob) run(ctx context.Context, admitted *ConversationHost) (remotehost.JobEnd, error) {
	if ctx.Err() != nil {
		return remotehost.JobEnd{}, errNotStarted
	}
	if !admitted.Host.Online() {
		return remotehost.JobEnd{}, fmt.Errorf("host %s is offline", j.target.Device)
	}
	if err := installDirectories(ctx, j.shard, j.target.Device, admitted); err != nil {
		return remotehost.JobEnd{}, err
	}
	defer j.shard.JobEnded(j.id)
	start := remotehost.JobStart{
		Script:   j.script,
		CWD:      j.target.Path,
		Env:      map[string]string{},
		Context:  j.invocation.Context,
		Caller:   j.invocation.Caller,
		Commands: j.shard.Commands().SelectionOf(string(j.invocation.Caller.Node), j.id),
		Stdout:   new(j.stdout.WireRef()),
	}
	if j.stdin != nil {
		start.Stdin = new(j.stdin.WireRef())
	}
	job, err := admitted.Host.StartJob(ctx, start)
	if err != nil {
		return remotehost.JobEnd{}, err
	}
	cleanup := context.WithoutCancel(ctx)
	defer func() {
		if err := job.Release(cleanup); err != nil {
			slog.Debug("the job release failed", "error", err)
		}
	}()
	relayCtx, cancel := context.WithCancel(ctx)
	var relays sync.WaitGroup
	relays.Add(1)
	go func() {
		defer relays.Done()
		relayJobOutput(ctx, cleanup, job, j.port)
	}()
	if j.stdin == nil {
		relays.Add(1)
		go func() {
			defer relays.Done()
			relayJobInput(relayCtx, cleanup, job, j.port)
		}()
	}
	end, err := endHostJob(ctx, cleanup, job, j.stdin, j.stdout)
	cancel()
	relays.Wait()
	if err != nil {
		return remotehost.JobEnd{}, err
	}
	return end, nil
}

func relayJobOutput(ctx, cleanup context.Context, job *remotehost.Job, port host.RPCPort) {
	for {
		output, err := job.NextOutput(cleanup)
		if err != nil {
			return
		}
		if output.Stream == "stderr" && len(output.Bytes) > 0 {
			if err := port.Stderr(ctx, output.Bytes); err != nil {
				return
			}
		}
	}
}

func relayJobInput(ctx, cleanup context.Context, job *remotehost.Job, port host.RPCPort) {
	for {
		bytes, err := port.ReadLiveStdin(ctx)
		if err != nil {
			return
		}
		if bytes == nil {
			_ = job.CloseStdin(cleanup)
			return
		} // An ended job already closed its input.
		if err := job.WriteStdin(ctx, bytes); err != nil {
			return
		}
	}
}

// endHostJob sends SIGTERM when ctx ends, waits five seconds, and sends SIGKILL only if that wait fails.
func endHostJob(
	ctx, cleanup context.Context,
	job *remotehost.Job,
	stdin, stdout *remotehost.Pipe,
) (remotehost.JobEnd, error) {
	end, err := job.End(ctx)
	if err != nil && ctx.Err() != nil {
		_ = job.Kill(cleanup, runnerproto.Signal("SIGTERM")) // An already-ended job needs no signal.
		stdout.Fail("command aborted")
		if stdin != nil {
			stdin.Fail("command aborted")
		}
		grace, stop := context.WithTimeout(cleanup, 5*time.Second)
		end, err = job.End(grace)
		stop()
		if err != nil {
			_ = job.Kill(cleanup, runnerproto.Signal("SIGKILL")) // A concurrent end makes killing unnecessary.
			end, err = job.End(cleanup)
		}
	}
	return end, err
}

func hostJobStatus(ctx context.Context, port host.RPCPort, status host.ProcessEnd) (uint8, error) {
	if status.Kind == host.ProcessNotStarted {
		detail := ""
		if status.SpawnError.Detail != nil {
			detail = " — " + *status.SpawnError.Detail
		}
		_ = port.Stderr(
			ctx,
			[]byte(fmt.Sprintf("host shell: %s%s\n", status.SpawnError.Kind, detail)),
		) // A departed caller reads nothing further.
		return 127, nil
	}
	if ctx.Err() != nil {
		return 130, nil
	}
	switch status.Kind {
	case host.ProcessExited:
		if status.ExitCode >= 0 && status.ExitCode <= 255 {
			return uint8(status.ExitCode), nil
		}
		return 1, nil
	case host.ProcessLost:
		return 0, errors.New(status.Reason)
	case host.ProcessSignalled:
		return 1, nil
	}
	return 1, nil
}

func bindHostPipes(
	shard HostShard,
	target ReachableHost,
	invocation host.RPCInvocation,
) (*remotehost.Pipe, *remotehost.Pipe, error) {
	if invocation.Caller == nil {
		return nil, nil, errors.New("host shell runs for an agent")
	}
	if invocation.Pipes == nil {
		return nil, nil, errors.New("cross-host execution requires a machine job")
	}
	stdout, ok := shard.Pipes().Pipe(invocation.Pipes.Stdout)
	if !ok {
		return nil, nil, errors.New("the command's standard output is gone")
	}
	var stdin *remotehost.Pipe
	if invocation.Pipes.Stdin != nil {
		stdin, ok = shard.Pipes().Pipe(*invocation.Pipes.Stdin)
		if !ok {
			return nil, nil, errors.New("the command's standard input is gone")
		}
		if err := stdin.SinkTo(string(target.Device)); err != nil {
			return nil, nil, err
		}
	}
	if err := stdout.SourceFrom(string(target.Device)); err != nil {
		return nil, nil, err
	}
	return stdin, stdout, nil
}
