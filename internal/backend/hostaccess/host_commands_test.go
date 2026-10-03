package hostaccess

import (
	"context"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
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
		invocation := host.RPCInvocation{Path: []string{"host", verb}, Args: []byte(args), Context: commandwire.CommandContext{Conversation: string(record.ID)}}
		code, err := commands.Dispatch(t.Context(), invocation, host.NewRPCPort(memory))
		return code, string(memory.Stdout()), err
	}
	if code, out, err := run("list", `{}`); err != nil || code != 0 || out != "Cloud has not been allocated\n" {
		t.Fatalf("list: %d %q %v", code, out, err)
	}
	if _, out, err := run("current", `{}`); err != nil || out != "host: Cloud (not allocated), directory /home/demi/sessions/"+string(record.ID)+"\n" {
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
