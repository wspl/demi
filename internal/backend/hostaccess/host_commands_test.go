package hostaccess

import (
	"context"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerwire"
)

func TestHostCommandsUseConversationAndStopWithShard(t *testing.T) {
	s := newTestShard(t)
	record := s.conversation(t)
	commands := &host.CommandSet{}
	if err := commands.Register(HostGroup(s)); err != nil {
		t.Fatal(err)
	}
	help := commands.RenderHelp()
	if !strings.Contains(help, hostSummary) || !strings.Contains(help, shellSummary) {
		t.Fatal("host command help differs from reference")
	}
	run := func(verb, args string) (uint8, string, error) {
		memory := hosttest.NewMemoryPort(nil)
		invocation := host.RPCInvocation{
			Path:    []string{"host", verb},
			Args:    []byte(args),
			Context: commandwire.CommandContext{Conversation: string(record.ID)},
		}
		code, err := commands.Dispatch(t.Context(), invocation, host.NewRPCPort(memory))
		return code, string(memory.Stdout()), err
	}
	if code, out, err := run("list", `{}`); err != nil || code != 0 || out != "Cloud has not been allocated\n" {
		t.Fatalf("list: %d %q %v", code, out, err)
	}
	if _, out, err := run(
		"current",
		`{}`,
	); err != nil ||
		out != "host: Cloud (not allocated), directory /home/demi/sessions/"+string(record.ID)+"\n" {
		t.Fatalf("current: %q %v", out, err)
	}
	if _, _, err := run("list", `{"unexpected":true}`); err == nil {
		t.Fatal("unknown command input accepted")
	}
	if err := s.conversations.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, _, err := run("list", `{}`); err == nil || !strings.Contains(err.Error(), "the backend is shutting down") {
		t.Fatal(err)
	}
}

// Cost: scripted runner frames and local storage; no shell process is started.
func TestConnectedCrossHostCommandInstallsAndCarriesExitAndDirectory(t *testing.T) {
	s := newTestShard(t)
	main := s.paired(t, "main")
	record := s.target(t, s.conversation(t), main, "/work")
	device := s.paired(t, "attached")
	if _, err := s.control.ChangeConversation(
		t.Context(),
		record.ID,
		&database.RecordAttach{Host: database.AttachedHostRecord{Device: device.ID, Name: "remote"}},
	); err != nil {
		t.Fatal(err)
	}
	r := connectHost(t, s, device)
	s.directories = DirectorySets{{Plugin: "test"}}
	commands := &host.CommandSet{}
	if err := commands.Register(HostGroup(s)); err != nil {
		t.Fatal(err)
	}
	commandContext, err := runners.CommandContext(
		t.Context(),
		s.control,
		s.owner,
		record.ID,
		&commandwire.AgentCaller{Number: 1},
	)
	if err != nil {
		t.Fatal(err)
	}
	input := s.pipes.Mint(nil, nil)
	writer, err := input.Writer()
	if err != nil {
		t.Fatal(err)
	}
	writer.End()
	output := s.pipes.Mint(nil, nil)
	reader, err := output.Reader()
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = reader.Close(context.Background()) }()
	defer input.Fail("test ended")
	args, err := (shellArgs{Host: "remote", Script: "exit 7"}).MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	invocation := host.RPCInvocation{
		Path:    []string{"host", "shell"},
		Args:    args,
		Context: commandContext,
		Caller:  &host.JobCaller{Node: RootOf(record.ID)},
		Pipes:   &host.RelayedPipes{Stdin: new(input.ID()), Stdout: output.ID()},
	}
	port := hosttest.NewMemoryPort(nil)
	result := startHostOperation(t, func(ctx context.Context) (uint8, error) {
		return commands.Dispatch(ctx, invocation, host.NewRPCPort(port))
	})
	listing := nextHostMessage[*runnerwire.FSReaddir](t, r)
	if listing.Path != "/home/test/.demi/plugins/test" {
		t.Fatal(listing.Path)
	}
	if hold := s.conversations.Slot(record.ID).FileGate().TryReserve(); hold != nil {
		hold.Release()
		t.Fatal("cross-host install escaped admission")
	}
	r.send(t, &runnerwire.FSOK{ID: listing.ID, Result: &runnerwire.FSReaddirResult{Value: []runnerwire.DirEntry{}}})
	job := nextHostMessage[*runnerwire.JobStart](t, r)
	if job.CWD != "/home/test" || job.Stdin == nil || job.Stdin.ID != input.ID() || job.Stdout == nil ||
		job.Stdout.ID != output.ID() {
		t.Fatal(job)
	}
	r.send(t, &runnerwire.JobOutput{JobID: job.JobID, Stream: runnerwire.Stderr, Bytes: []byte("remote warning\n")})
	r.send(
		t,
		&runnerwire.JobExit{
			JobID:    job.JobID,
			ExitCode: new(int32(7)),
			CWD:      new("/next"),
			Files:    []runnerwire.JobFileChange{},
		},
	)
	_ = nextHostMessage[*runnerwire.JobRelease](t, r)
	completed := <-result
	if completed.err != nil || completed.value != 7 || string(port.Stderr()) != "remote warning\n" {
		t.Fatal(completed, string(port.Stderr()))
	}
	attached, err := s.control.AttachedHosts(t.Context(), record.ID)
	if err != nil || len(attached) != 1 || attached[0].CWD == nil || *attached[0].CWD != "/next" {
		t.Fatal(attached, err)
	}
	if s.jobs != 1 {
		t.Fatal("cross-host job omitted completion")
	}
}
