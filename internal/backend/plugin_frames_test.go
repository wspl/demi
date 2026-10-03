package backend_test

import (
	"context"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// filesView waits for a page's unsolicited view of one shell command.
func filesView(
	ctx context.Context,
	t *testing.T,
	page *backendtest.ConversationSocket,
	id core.CommandID,
	wanted func(framewire.ShellStatus) bool,
) framewire.ShellStatus {
	t.Helper()
	frames, err := page.Until(ctx, func(f framewire.ServerFrame) bool {
		v, ok := f.(*framewire.ShellOutputFrame)
		return ok && v.Status.Command().CommandID == id && wanted(v.Status)
	})
	wireMust(t, err)
	return frames[len(frames)-1].(*framewire.ShellOutputFrame).Status
}

// filesEnded selects terminal shell views without consuming the model's cursor.
func filesEnded(s framewire.ShellStatus) bool {
	_, running := s.(*framewire.RunningStatus)
	return !running
}

// TestPluginFramesPageOutputRemainsForModel uses a real reader job waiting for page input; the model then reads
// its independent cursor.
func TestPluginFramesPageOutputRemainsForModel(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "")
	w.vendor.Respond(conversationShell(t, "later", "read go; echo later; read line", 100))
	w.vendor.Respond(conversationAnswer(t, []string{"started"}, 1, 1))
	_, err := w.socket.Chat(w.ctx, "message-1", "Start it.")
	wireMust(t, err)
	id := core.CommandID(toolstest.Field(conversationToolResult(t, w.vendor.Requests()[1], "later"), "commandId"))
	wireMust(t, w.socket.Send(w.ctx, &framewire.ShellWriteFrame{CommandID: id, Stdin: "go\n"}))
	filesView(w.ctx, t, w.socket, id, func(s framewire.ShellStatus) bool { return s.Command().Tail == "later\n" })
	w.vendor.Respond(conversationToolUse(t, "check", "shell_status", `{"commandId":"`+string(id)+`"}`))
	w.vendor.Respond(conversationAnswer(t, []string{"checked"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "message-2", "Check it.")
	wireMust(t, err)
	requests := w.vendor.Requests()
	filesContains(t, toolstest.ShownOutput(conversationToolResult(t, requests[len(requests)-1], "check")), "later")
	wireMust(t, w.socket.Send(w.ctx, &framewire.ShellAbortFrame{CommandID: id}))
	filesView(w.ctx, t, w.socket, id, filesEnded)
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestPluginFramesChattyCommandKeepsIdlePageConnected writes 140 KB with a real shell while one page reads
// nothing; both retain its end.
func TestPluginFramesChattyCommandKeepsIdlePageConnected(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "", func(h *backendtest.Harness) { h.Config.Conversations.OutboxFrames = 64 })
	idle := conversationOpen(w.ctx, t, w.backend, &w.session, filesConversation)
	w.vendor.Respond(
		conversationShell(
			t,
			"chatty",
			"i=0; while [ $i -lt 1000 ]; do echo $i; i=$((i+1)); done; seq 100000 120000; echo beyond; read done",
			200,
		),
	)
	w.vendor.Respond(conversationAnswer(t, []string{"chatting"}, 1, 1))
	started := time.Now()
	frames, err := w.socket.Chat(w.ctx, "message-1", "Chat.")
	wireMust(t, err)
	id := core.CommandID(toolstest.Field(conversationToolResult(t, w.vendor.Requests()[1], "chatty"), "commandId"))
	beyond := func(s framewire.ShellStatus) bool {
		_, running := s.(*framewire.RunningStatus)
		return running && strings.Contains(s.Command().Tail, "beyond\n")
	}
	seen := false
	for _, f := range frames {
		if v, ok := f.(*framewire.ShellOutputFrame); ok && v.Status.Command().CommandID == id && beyond(v.Status) {
			seen = true
		}
	}
	if !seen {
		filesView(w.ctx, t, w.socket, id, beyond)
	}
	wireMust(t, w.socket.Send(w.ctx, &framewire.ShellWriteFrame{CommandID: id, Stdin: "done\n"}))
	filesView(w.ctx, t, w.socket, id, filesEnded)
	elapsed := time.Since(started)
	views, err := idle.Until(w.ctx, func(f framewire.ServerFrame) bool {
		v, ok := f.(*framewire.ShellOutputFrame)
		return ok && v.Status.Command().CommandID == id && filesEnded(v.Status)
	})
	wireMust(t, err)
	count := 0
	for _, f := range views {
		if v, ok := f.(*framewire.ShellOutputFrame); ok && v.Status.Command().CommandID == id {
			count++
		}
	}
	if _, ok := views[len(views)-1].(*framewire.ShellOutputFrame).Status.(*framewire.ExitedStatus); !ok {
		t.Fatal("idle page missed normal end")
	}
	if count > int(elapsed/(250*time.Millisecond))+2 {
		t.Fatalf("%d views in %s", count, elapsed)
	}
	wireMust(t, idle.Close(w.ctx))
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}

// TestPluginFramesEveryPageSeesOutputAndCommandEnd uses four jobs to exercise unsolicited output, two pages,
// input, abort and close cleanup.
func TestPluginFramesEveryPageSeesOutputAndCommandEnd(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "")
	w.vendor.Respond(conversationShell(t, "greeter", "echo hello", 30000))
	w.vendor.Respond(conversationAnswer(t, []string{"greeted"}, 1, 1))
	frames, err := w.socket.Chat(w.ctx, "message-1", "Greet.")
	wireMust(t, err)
	greeted, returned := -1, -1
	for i, f := range frames {
		if v, ok := f.(*framewire.ShellOutputFrame); ok {
			if end, ok := v.Status.(*framewire.ExitedStatus); ok && end.ExitCode == 0 && end.Tail == "hello\n" {
				greeted = i
				conversationEqual(t, v.SubagentID, (*core.NodeID)(nil))
				conversationEqual(t, end.ToolUseID, "greeter")
			}
		}
		if p, ok := f.(*framewire.TranscriptPatchFrame); ok {
			for _, patch := range p.Patches {
				if replace, ok := patch.(*framewire.ReplaceBlockPatch); ok {
					if call, ok := replace.Value.(*core.ToolCallBlock); ok && call.ToolUseID == "greeter" &&
						string(call.Status) != "executing" {
						returned = i
					}
				}
			}
		}
	}
	if greeted < 0 || returned < 0 || greeted >= returned {
		t.Fatalf("end index %d, result index %d", greeted, returned)
	}
	w.vendor.Respond(conversationShell(t, "reader", `read name; echo "hello $name"`, 200))
	w.vendor.Respond(conversationAnswer(t, []string{"waiting for a name"}, 1, 1))
	_, err = w.socket.Chat(w.ctx, "message-2", "Ask for a name.")
	wireMust(t, err)
	requests := w.vendor.Requests()
	reader := core.CommandID(
		toolstest.Field(conversationToolResult(t, requests[len(requests)-1], "reader"), "commandId"),
	)
	wireMust(t, w.socket.Send(w.ctx, &framewire.ShellWriteFrame{CommandID: reader, Stdin: "Alice\n"}))
	answers, err := w.socket.Until(w.ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.ShellWriteResultFrame)
		return ok
	})
	wireMust(t, err)
	var end framewire.ShellStatus
	for _, f := range answers {
		if _, ok := f.(*framewire.ErrorFrame); ok {
			t.Fatalf("write: %+v", f)
		}
		if v, ok := f.(*framewire.ShellOutputFrame); ok && v.Status.Command().CommandID == reader &&
			filesEnded(v.Status) {
			end = v.Status
		}
	}
	if end == nil {
		end = filesView(w.ctx, t, w.socket, reader, filesEnded)
	}
	exited, ok := end.(*framewire.ExitedStatus)
	if !ok || exited.ExitCode != 0 || exited.Tail != "hello Alice\n" {
		t.Fatalf("reader end: %+v", end)
	}
	w.vendor.Respond(conversationShell(t, "long", "echo long-ready; read go; echo went; sleep 30", 200))
	w.vendor.Respond(conversationAnswer(t, []string{"waiting"}, 1, 1))
	frames, err = w.socket.Chat(w.ctx, "message-3", "Wait.")
	wireMust(t, err)
	requests = w.vendor.Requests()
	long := core.CommandID(toolstest.Field(conversationToolResult(t, requests[len(requests)-1], "long"), "commandId"))
	ready := false
	for _, f := range frames {
		if v, ok := f.(*framewire.ShellOutputFrame); ok && v.Status.Command().CommandID == long &&
			v.Status.Command().Tail == "long-ready\n" {
			ready = true
		}
	}
	if !ready {
		filesView(
			w.ctx,
			t,
			w.socket,
			long,
			func(s framewire.ShellStatus) bool { return s.Command().Tail == "long-ready\n" },
		)
	}
	second, err := w.backend.Conversation(w.ctx, t, &w.session, filesConversation)
	wireMust(t, err)
	_, err = second.Open(w.ctx)
	wireMust(t, err)
	var live []string
	// Open ends at its protocol marker; the pending command views follow it.
	for len(live) < 2 {
		f, e := second.Next(w.ctx)
		wireMust(t, e)
		if v, ok := f.(*framewire.ShellOutputFrame); ok {
			live = append(live, string(v.Status.Command().CommandID)+":"+v.Status.Command().Tail)
			if len(live) == 1 && !filesEnded(v.Status) {
				t.Fatal("reader still running")
			}
			if len(live) == 2 && filesEnded(v.Status) {
				t.Fatal("long ended")
			}
		}
	}
	conversationEqual(t, live, []string{string(reader) + ":hello Alice\n", string(long) + ":long-ready\n"})
	wireMust(t, w.socket.Send(w.ctx, &framewire.ShellWriteFrame{CommandID: long, Stdin: "go\n"}))
	for _, page := range []*backendtest.ConversationSocket{w.socket, second} {
		filesView(
			w.ctx,
			t,
			page,
			long,
			func(s framewire.ShellStatus) bool { return strings.HasSuffix(s.Command().Tail, "went\n") },
		)
	}
	wireMust(t, second.Send(w.ctx, &framewire.ShellAbortFrame{CommandID: long}))
	for _, page := range []*backendtest.ConversationSocket{w.socket, second} {
		if _, ok := filesView(w.ctx, t, page, long, filesEnded).(*framewire.AbortedStatus); !ok {
			t.Fatal("long was not aborted")
		}
	}
	wireMust(t, second.Send(w.ctx, &framewire.ShellWriteFrame{CommandID: long, Stdin: "late\n"}))
	refused, err := second.Next(w.ctx)
	wireMust(t, err)
	if _, ok := refused.(*framewire.ErrorFrame); !ok {
		t.Fatalf("late write: %T", refused)
	}
	w.vendor.Respond(conversationShell(t, "sleeper", "sh -c 'echo $$ > ../sleeper.pid; exec sleep 30'", 200))
	w.vendor.Respond(conversationAnswer(t, []string{"sleeping"}, 1, 1))
	frames, err = second.Chat(w.ctx, "message-4", "Start the sleeper.")
	wireMust(t, err)
	pidPath := filepath.Join(w.paired.Runner.Home(), "sleeper.pid")
	wireMust(t, backendtest.WaitFile(w.ctx, pidPath, func(b []byte) bool { return strings.HasSuffix(string(b), "\n") }))
	wireMust(t, second.Send(w.ctx, &framewire.CloseFrame{}))
	closing, err := second.Until(w.ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.ClosedFrame)
		return ok
	})
	wireMust(t, err)
	for _, f := range append(frames, closing...) {
		if v, ok := f.(*framewire.ShellOutputFrame); ok &&
			(v.Status.Command().CommandID == reader || v.Status.Command().CommandID == long) {
			t.Fatalf("output after end: %+v", v)
		}
	}
	_, err = w.socket.Until(w.ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.ClosedFrame)
		return ok
	})
	wireMust(t, err)
	wireMust(t, backendtest.WaitRunnerJobsRemoved(w.ctx, w.paired.Runner.StateDir()))
	pid, err := strconv.Atoi(strings.TrimSpace(filesRead(t, pidPath)))
	wireMust(t, err)
	if running, err := backendtest.ProcessRunning(pid); err != nil || running {
		t.Fatalf("sleeper remains: %v", err)
	}
	wireMust(t, second.Close(w.ctx))
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}
