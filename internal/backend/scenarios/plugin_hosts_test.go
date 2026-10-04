package scenarios_test

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
)

// TestPluginHostsKeepShellsAndRefuseHandlesOnOtherHost uses one runner to supply two working directories; four
// jobs observe distinct Host shells.
func TestPluginHostsKeepShellsAndRefuseHandlesOnOtherHost(t *testing.T) {
	t.Parallel()
	w := filesWorking(t, "")
	alice, bob := filepath.Join(w.root, "alice"), filepath.Join(w.root, "bob")
	wireMust(t, os.MkdirAll(alice, 0o755))
	wireMust(t, os.MkdirAll(bob, 0o755))
	for i, step := range []struct {
		path, script, want string
	}{
		{
			alice,
			"mkdir nested && cd nested && pwd",
			alice + "/nested\n",
		},
		{
			bob,
			"pwd",
			bob + "\n",
		},
		{
			alice,
			"pwd",
			alice + "/nested\n",
		},
	} {
		filesMove(w.ctx, t, w.backend, &w.session, filesConversation, w.paired, step.path)
		id := string(rune('a' + i))
		w.vendor.Respond(conversationShell(t, id, step.script, 30000))
		w.vendor.Respond(conversationAnswer(t, []string{"done"}, 1, 1))
		_, err := w.socket.Chat(w.ctx, string(rune('a'+i)), "Where?")
		wireMust(t, err)
		requests := w.vendor.Requests()
		conversationEqual(t, toolstest.ShownOutput(conversationToolResult(t, requests[len(requests)-1], id)), step.want)
	}
	w.vendor.Respond(conversationShell(t, "reader", `read name; echo "hello $name"`, 200))
	w.vendor.Respond(conversationAnswer(t, []string{"waiting for a name"}, 1, 1))
	_, err := w.socket.Chat(w.ctx, "message-4", "Ask Alice for a name.")
	wireMust(t, err)
	requests := w.vendor.Requests()
	id := types.CommandID(toolstest.Field(conversationToolResult(t, requests[len(requests)-1], "reader"), "commandId"))
	db, err := w.harness.ControlDatabase(w.ctx, t)
	wireMust(t, err)
	// Change the conversation's Host in the database while the job lives; the public route forbids it.
	_, err = db.ExecContext(w.ctx, "UPDATE conversations SET target_path=? WHERE id=?", bob, filesConversation)
	wireMust(t, err)
	wireMust(t, w.socket.Send(w.ctx, &conversationproto.ShellWriteFrame{CommandID: id, Stdin: "wrong\n"}))
	frames, err := w.socket.Until(w.ctx, func(f conversationproto.ServerFrame) bool {
		_, ok := f.(*conversationproto.ErrorFrame)
		return ok
	})
	wireMust(t, err)
	filesContains(t, frames[len(frames)-1].(*conversationproto.ErrorFrame).Message, "belongs to a different Host")
	_, err = db.ExecContext(w.ctx, "UPDATE conversations SET target_path=? WHERE id=?", alice, filesConversation)
	wireMust(t, err)
	wireMust(t, w.socket.Send(w.ctx, &conversationproto.ShellWriteFrame{CommandID: id, Stdin: "right\n"}))
	frames, err = w.socket.Until(w.ctx, func(f conversationproto.ServerFrame) bool {
		_, refused := f.(*conversationproto.ErrorFrame)
		_, written := f.(*conversationproto.ShellWriteResultFrame)
		return refused || written
	})
	wireMust(t, err)
	if _, ok := frames[len(frames)-1].(*conversationproto.ShellWriteResultFrame); !ok {
		t.Fatalf("write: %+v", frames)
	}
	wireMust(t, w.socket.Close(w.ctx))
	wireMust(t, w.backend.Close(w.ctx))
}
