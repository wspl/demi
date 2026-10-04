package server_test

import (
	"context"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/server/servertest"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider/providertest"
)

type scriptedHost struct{ toolstest.NoHost }

func (*scriptedHost) Key() host.Key { return host.Key("scripted") }

func (*scriptedHost) DefaultCWD() string { return "/workspace" }

func (*scriptedHost) Identity() host.Identity { return host.Identity{Hostname: "scripted"} }

func (h *scriptedHost) Host(context.Context, tools.NodeContext) (*scriptedHost, error) { return h, nil }

type scriptedShell struct {
	mu      sync.Mutex
	feed    host.PageFeed
	command *host.CommandRecord
}

func (s *scriptedShell) Exec(_ context.Context, r host.ExecRequest) (host.CommandStatus, error) {
	record := host.NewCommandRecord("1", "1", r.ToolUseID)
	s.mu.Lock()
	s.command = record
	s.mu.Unlock()
	s.feed.Changed(record)
	return record.Status(host.DefaultOutputLimitBytes, nil), nil
}

func (s *scriptedShell) record() *host.CommandRecord {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.command
}

func (s *scriptedShell) print(text string) {
	r := s.record()
	r.AppendOutput(core.StreamKindStdout, text)
	s.feed.Changed(r)
}

func (s *scriptedShell) end() {
	r := s.record()
	if r != nil {
		r.MarkAborted()
		s.feed.Changed(r)
	}
}

func (s *scriptedShell) Status(core.CommandID) (host.CommandStatus, error) {
	return s.record().Status(host.DefaultOutputLimitBytes, nil), nil
}

func (*scriptedShell) ReadOutput(context.Context, core.CommandID) (host.WholeOutput, error) {
	return host.WholeOutput{}, &host.ShellError{Kind: host.UnknownCommand}
}

func (*scriptedShell) Write(context.Context, core.CommandID, []byte) error {
	return &host.ShellError{Kind: host.NotRunning}
}

func (*scriptedShell) Abort(context.Context, core.CommandID) error {
	return &host.ShellError{Kind: host.NotRunning}
}

func (s *scriptedShell) PageViews() []host.PageView {
	r := s.record()
	if r == nil {
		return nil
	}
	return []host.PageView{r.PageView()}
}
func (*scriptedShell) ReleaseCommand(context.Context, core.CommandID) bool { return false }
func (*scriptedShell) DisposeShell(context.Context, core.ShellID) bool     { return false }
func (s *scriptedShell) DisposeAll(context.Context) error {
	s.end()
	return nil
}

func (s *scriptedShell) OwnsShell(id core.ShellID) bool { return s.record() != nil && id == "1" }

func (s *scriptedShell) OwnsCommand(id core.CommandID) bool { return s.record() != nil && id == "1" }

type scriptedShells struct {
	mu    sync.Mutex
	shell *scriptedShell
}

func (s *scriptedShells) Create(
	_ context.Context,
	scope tools.EnvironmentScope,
	_ *scriptedHost,
) (host.ShellEnvironment, error) {
	shell := &scriptedShell{feed: scope.Feed}
	s.mu.Lock()
	s.shell = shell
	s.mu.Unlock()
	return shell, nil
}

func serving(t *testing.T) (*server.Server[*scriptedHost], *scriptedShell, *servertest.TestClient[*scriptedHost]) {
	t.Helper()
	script := providertest.NewScriptedRuntime(
		t,
		providertest.Events(
			providertest.ToolCall("call-1", "shell_exec", []byte(`{"script":"serve","timeoutMs":200}`)),
		),
		said("serving"),
	)
	providers := &servertest.ScriptedProviders{}
	providers.Provide("stub", script)
	memory := storetest.NewMemoryTreeStore()
	shells := &scriptedShells{}
	s := server.New(
		server.Deps[*scriptedHost]{
			Toolsets:      tools.Set{Commands: &host.CommandSet{}, Revision: "none"},
			Instructions:  "system prompt",
			Hosts:         &scriptedHost{},
			Shells:        shells,
			Providers:     providers,
			Stores:        func(core.NodeID) store.Tree { return memory },
			Clock:         core.SystemClock{},
			IDs:           &testIDs{},
			Config:        server.DefaultConfig(),
			StatusChanged: func(core.NodeID) {},
		},
	)
	t.Cleanup(func() {
		if err := s.Shutdown(context.Background()); err != nil {
			t.Error(err)
		}
	})
	c := servertest.Connect(t, s, rootID(), "/workspace")
	c.Send(t.Context(), &framewire.OpenFrame{})
	c.Received()
	c.Send(t.Context(), send("m1", "Serve."))
	if _, err := c.NextUntil(t.Context(), func(f framewire.ServerFrame) bool {
		p, ok := f.(*framewire.PhaseFrame)
		return ok && p.Phase == core.SessionPhaseIdle
	}); err != nil {
		t.Fatal(err)
	}
	shells.mu.Lock()
	shell := shells.shell
	shells.mu.Unlock()
	if shell == nil {
		t.Fatalf("no shell: %#v", s.Tree(rootID()).Root().Session().Transcript().Blocks)
	}
	return s, shell, c
}

func tails(t *testing.T, frames []framewire.ServerFrame) []string {
	result := []string{}
	for _, frame := range frames {
		if s, ok := frame.(*framewire.ShellOutputFrame); ok {
			r, ok := s.Status.(*framewire.RunningStatus)
			if !ok {
				t.Fatalf("expected running shell status, got %T", s.Status)
			}
			result = append(result, r.Tail)
		}
	}
	return result
}

func TestDetachedOutputDoesNotReplayAsNewOutput(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s, shell, first := serving(t)
		shell.print("one\n")
		time.Sleep(250 * time.Millisecond)
		synctest.Wait()
		seen := tails(t, first.Received())
		equal(t, "one\n", seen[len(seen)-1])
		shell.print("two\n")
		first.Connection().Detach()
		synctest.Wait()
		shell.print("three\n")
		second := servertest.Connect(t, s, rootID(), "/workspace")
		second.Send(t.Context(), &framewire.OpenFrame{})
		equal(t, []string{"one\ntwo\nthree\n"}, tails(t, second.Received()))
		time.Sleep(500 * time.Millisecond)
		synctest.Wait()
		equal(t, []string{}, tails(t, second.Received()))
		shell.print("four\n")
		time.Sleep(250 * time.Millisecond)
		synctest.Wait()
		equal(t, []string{"one\ntwo\nthree\nfour\n"}, tails(t, second.Received()))
	})
}

func TestDetachedRunningCommandKeepsTreeLive(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s, shell, c := serving(t)
		c.Connection().Detach()
		synctest.Wait()
		time.Sleep(20 * time.Minute)
		if s.Tree(rootID()) == nil {
			t.Fatal("running command evicted")
		}
		shell.end()
		synctest.Wait()
		time.Sleep(10*time.Minute - time.Second)
		if s.Tree(rootID()) == nil {
			t.Fatal("evicted early")
		}
		time.Sleep(2 * time.Second)
		synctest.Wait()
		if s.Tree(rootID()) != nil {
			t.Fatal("quiet command kept tree live")
		}
	})
}
