package remotehost_test

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"strings"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/cmddecl"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerproto"
)

// relayCommands binds the probe leaf to a scenario-owned handler.
func relayCommands(t *testing.T, handler host.RPCHandlerFunc) *host.CommandSet {
	t.Helper()
	commands := &host.CommandSet{}
	requirePipe(
		t,
		commands.Register(
			host.Group(
				"probe",
				"Probes.",
				host.Leaf(
					cmddecl.Leaf[cmddecl.NativeOperation]{
						Name:    "live",
						Summary: "Live input.",
						Kind:    &cmddecl.RPC[cmddecl.NativeOperation]{},
					},
					handler,
				),
			),
		),
	)
	return commands
}

// rpcRequest names the job whose context must authorize and reach the handler.
func rpcRequest(job, id string) *runnerproto.RPCCall {
	return &runnerproto.RPCCall{
		JobID:  job,
		CallID: id,
		Root:   "probe",
		Path:   []string{"probe", "live"},
		Argv:   []string{"live"},
		Args:   json.RawMessage(`{}`),
		CWD:    "/work",
		Env:    map[string]string{},
		Stdin:  true,
	}
}

// callOutcome observes stderr then exit and rejects unexpected pipe allocation.
func callOutcome(t *testing.T, l *remotehosttest.TestLink) (string, uint8) {
	t.Helper()
	var stderr strings.Builder
	for {
		frame := nextFrame(t, l)
		if output, ok := frame.(*runnerproto.RPCOutput); ok {
			stderr.Write(output.Bytes)
			continue
		}
		if exit, ok := frame.(*runnerproto.RPCExit); ok {
			return stderr.String(), exit.ExitCode
		}
		t.Fatalf("unexpected RPC frame %T", frame)
	}
}

func TestCallStopsOnFirstCauseReleasesLiveInputAndExitsAfterHandler(t *testing.T) {
	for _, event := range []string{
		"cancel",
		"job",
		"report",
		"stdout",
		"stdin",
		"chunk",
		"unasked",
		"disconnect",
		"shutdown",
	} {
		t.Run(event, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				ended := make(chan struct{})
				observed := make(chan host.RPCInvocation, 1)
				handler := host.RPCHandlerFunc(
					func(ctx context.Context, inv host.RPCInvocation, port host.RPCPort) (uint8, error) {
						defer close(ended)
						observed <- inv
						for {
							data, err := port.ReadLiveStdin(ctx)
							if err != nil || data == nil {
								break
							}
							if event == "unasked" {
								<-ctx.Done()
								break
							}
						}
						<-ctx.Done()
						return 0, nil
					},
				)
				d := remotehosttest.NewTestDevice(t, remotehosttest.NewCommandPolicy(relayCommands(t, handler)))
				l := d.Connect(0)
				request := startRequest("probe live")
				request.Caller = &host.JobCaller{Node: "test-session"}
				job, err := d.Host("/work", nil).StartJob(t.Context(), request)
				requirePipe(t, err)
				nextFrame(t, l)
				sendFrame(t, l, rpcRequest(job.ID(), "call"))
				pipes := nextFrame(t, l).(*runnerproto.RPCPipes)
				if pipes.Stdin == nil {
					t.Fatal("missing stdin pipe")
				}
				_ = nextFrame(t, l).(*runnerproto.RPCStdinPull)
				expected := ""
				switch event {
				case "cancel":
					sendFrame(t, l, &runnerproto.RPCCancel{CallID: "call"})
					expected = "command cancelled"
				case "job":
					sendFrame(
						t,
						l,
						&runnerproto.JobExit{
							JobID:    job.ID(),
							ExitCode: new(int32(7)),
							Files:    []runnerproto.JobFileChange{},
						},
					)
					expected = fmt.Sprintf("calling job %s exited before its RPC completed", job.ID())
				case "report":
					sendFrame(
						t,
						l,
						&runnerproto.PipeDone{PipeID: pipes.Stdout.ID, Ok: false, Error: new("upload failed")},
					)
					expected = "pipe failed: upload failed"
				case "stdout", "stdin":
					id := pipes.Stdout.ID
					if event == "stdin" {
						id = pipes.Stdin.ID
					}
					d.Pipes().Fail(id, "HTTP connection lost")
					expected = "pipe failed: HTTP connection lost"
				case "chunk":
					sendFrame(
						t,
						l,
						&runnerproto.RPCStdin{CallID: "call", Bytes: make([]byte, runnerproto.StdinChunkBytes+1)},
					)
					expected = "Unrequested or oversized RPC stdin chunk"
				case "unasked":
					for range 2 {
						sendFrame(t, l, &runnerproto.RPCStdin{CallID: "call", Bytes: []byte("typed")})
					}
					expected = "Unrequested or oversized RPC stdin chunk"
				case "disconnect":
					_, err = l.Close(t.Context())
					requirePipe(t, err)
				case "shutdown":
					l.Link().Disconnect("backend shutting down")
					end, err := l.Ended(t.Context())
					requirePipe(t, err)
					if end.Kind != remotehost.LinkDisconnected || end.Reason != "backend shutting down" {
						t.Fatal(end)
					}
				}
				if event == "disconnect" || event == "shutdown" {
					select {
					case <-ended:
					default:
						t.Fatal("handler outlived driver")
					}
					return
				}
				sendFrame(
					t,
					l,
					&runnerproto.JobExit{
						JobID:    job.ID(),
						ExitCode: new(int32(7)),
						Files:    []runnerproto.JobFileChange{},
					},
				)
				stderr, code := callOutcome(t, l)
				select {
				case <-ended:
				default:
					t.Fatal("exit preceded handler completion")
				}
				wantCode := uint8(1)
				if event == "cancel" {
					wantCode = 130
				}
				if stderr != "probe: "+expected+"\n" || code != wantCode {
					t.Fatalf("%q, %d", stderr, code)
				}
				inv := <-observed
				if inv.Context.Conversation != hosttest.CommandContext().Conversation ||
					!reflect.DeepEqual(inv.Caller, request.Caller) {
					t.Fatal(inv)
				}
			})
		})
	}
}

func TestCallExitsAfterStdoutDrained(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		commands := relayCommands(t, func(ctx context.Context, _ host.RPCInvocation, p host.RPCPort) (uint8, error) {
			return 3, p.Stdout(ctx, []byte("hello"))
		})
		d := remotehosttest.NewTestDevice(t, remotehosttest.NewCommandPolicy(commands))
		l := d.Connect(0)
		job, err := d.Host("/work", nil).StartJob(t.Context(), startRequest("probe live"))
		requirePipe(t, err)
		nextFrame(t, l)
		request := rpcRequest(job.ID(), "call")
		request.Stdin = false
		sendFrame(t, l, request)
		pipes := nextFrame(t, l).(*runnerproto.RPCPipes)
		if pipes.Stdin != nil {
			t.Fatal("unexpected stdin")
		}
		synctest.Wait()
		if l.Queued() != 0 {
			t.Fatal("exit before drain")
		}
		sink, err := d.Pipes().ClaimSink(pipes.Stdout.ID, remotehosttest.TestDeviceID)
		requirePipe(t, err)
		defer func() {
			requirePipe(t, sink.Close(context.Background()))
		}()
		requirePipe(t, sink.SourceArrived(t.Context()))
		data, err := collectPipe(t.Context(), sink)
		requirePipe(t, err)
		stderr, code := callOutcome(t, l)
		if string(data) != "hello" || stderr != "" || code != 3 {
			t.Fatalf("%q %q %d", data, stderr, code)
		}
	})
}

// refusingPolicy uses the fixture's unrelated services but denies RPC admission.
type refusingPolicy struct{ *remotehosttest.CommandPolicy }

func (refusingPolicy) AdmitCall(remotehost.JobOrigin) error {
	return errors.New("rpc job belongs to another conversation")
}

func TestCallRequiresAdmittedLiveJobAndRefusalMintsNoPipe(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		d := remotehosttest.NewTestDevice(t, refusingPolicy{remotehosttest.NewCommandPolicy(nil)})
		l := d.Connect(0)
		job, err := d.Host("/work", nil).StartJob(t.Context(), startRequest("probe live"))
		requirePipe(t, err)
		nextFrame(t, l)
		for _, item := range []struct {
			job, id, reason string
		}{
			{
				"invented",
				"unknown",
				"rpc requires a live job dispatched to this device",
			},
			{
				job.ID(),
				"refused",
				"rpc job belongs to another conversation",
			},
		} {
			sendFrame(t, l, rpcRequest(item.job, item.id))
			stderr, code := callOutcome(t, l)
			if stderr != "probe: "+item.reason+"\n" || code != 1 {
				t.Fatalf("%q %d", stderr, code)
			}
		}
		commands := relayCommands(t, func(ctx context.Context, _ host.RPCInvocation, _ host.RPCPort) (uint8, error) {
			<-ctx.Done()
			return 0, nil
		})
		another := remotehosttest.NewTestDevice(t, remotehosttest.NewCommandPolicy(commands))
		second := another.Connect(0)
		running, err := another.Host("/work", nil).StartJob(t.Context(), startRequest("probe live"))
		requirePipe(t, err)
		nextFrame(t, second)
		sendFrame(t, second, rpcRequest(running.ID(), "twice"))
		sendFrame(t, second, rpcRequest(running.ID(), "twice"))
		end, err := second.Ended(t.Context())
		requirePipe(t, err)
		if end.Kind != remotehost.LinkDisconnected || end.Reason != "duplicate rpc call twice" {
			t.Fatal(end)
		}
	})
}
