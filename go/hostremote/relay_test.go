package hostremote_test

import (
	"context"
	"encoding/json/jsontext"
	"errors"
	"testing"

	"github.com/wspl/demi/go/commandtree"
	"github.com/wspl/demi/go/hostremote/hostremotetest"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

// Cost: one fake connection; handler and pipe events control every transition.
func TestRPCPullsLiveInputAndKeepsTheFirstFailure(t *testing.T) {
	commands := &shell.CommandSet{}
	input := make(chan []byte, 1)
	exited := make(chan struct{})
	stopped := make(chan struct{})
	release := make(chan struct{})
	defer close(release)
	err := commands.Register(shell.DeclareLeaf(commandtree.Leaf{Name: "probe", Kind: commandtree.KindRPC}, shell.RPCHandlerFunc(func(ctx context.Context, _ shell.RPCInvocation, port shell.RPCPort) (uint8, error) {
		defer close(exited)
		bytes, err := port.ReadLiveStdin(ctx)
		if err != nil {
			return 0, err
		}
		input <- bytes
		<-ctx.Done()
		close(stopped)
		<-release
		return 0, errors.New("handler's later failure")
	})))
	if err != nil {
		t.Fatal(err)
	}
	device := hostremotetest.NewTestDevice(t, hostremotetest.NewCommandPolicy(commands))
	link := device.Connect(0)
	job, err := device.Host("/work", nil).StartJob(t.Context(), jobStart())
	if err != nil {
		t.Fatal(err)
	}
	next(t, link)
	call := runnerproto.OutboundRPCCall{JobID: job.ID(), CallID: "call", Stdin: true, Root: "probe", Path: []string{"probe"}, Argv: []string{}, Args: jsontext.Value(`{}`), Cwd: "/work", Env: map[string]string{}}
	send(t, link, call)
	pipes := next(t, link).(runnerproto.InboundRPCPipes)
	if pull, ok := next(t, link).(runnerproto.InboundRPCStdinPull); !ok || pull.CallID != "call" {
		t.Fatal("missing pull")
	}
	send(t, link, runnerproto.OutboundRPCStdin{CallID: "call", Bytes: []byte("live")})
	if string(<-input) != "live" {
		t.Fatal("lost live input")
	}
	// Pipe failure exists before cancellation; the cancellation must not hide it.
	if !device.Pipes.FailFromDevice(pipes.Stdin.ID, hostremotetest.TestDeviceID, "source lost") {
		t.Fatal("pipe failure refused")
	}
	<-stopped
	send(t, link, runnerproto.OutboundRPCCancel{CallID: "call"})
	synced := make(chan error, 1)
	go func() { synced <- link.Link.Sync(t.Context()) }()
	barrier := next(t, link).(runnerproto.InboundSync)
	send(t, link, runnerproto.OutboundSyncDone{ID: barrier.ID})
	if err := <-synced; err != nil {
		t.Fatal(err)
	}
	release <- struct{}{}
	<-exited
	diagnostic := next(t, link).(runnerproto.InboundRPCOutput)
	if string(diagnostic.Bytes) != "probe: pipe failed: source lost\n" {
		t.Fatalf("diagnostic %q", diagnostic.Bytes)
	}
	if exit := next(t, link).(runnerproto.InboundRPCExit); exit.ExitCode != 1 {
		t.Fatal(exit)
	}
	call.JobID = "absent"
	call.CallID = "orphan"
	send(t, link, call)
	diagnostic = next(t, link).(runnerproto.InboundRPCOutput)
	if string(diagnostic.Bytes) != "probe: rpc requires a live job dispatched to this device\n" {
		t.Fatal(string(diagnostic.Bytes))
	}
	if exit := next(t, link).(runnerproto.InboundRPCExit); exit.ExitCode != 1 {
		t.Fatal(exit)
	}
}
