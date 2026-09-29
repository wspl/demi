package backendtest_test

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// editOutcome is the outcome of the edit the frame asks for, which the socket
// answers.
func editOutcome(socket *backendtest.Socket, frame backendtest.Frame) map[string]any {
	socket.Send(frame)
	answered := socket.UntilType("edit_result")
	outcome, _ := answered[len(answered)-1]["outcome"].(map[string]any)
	return outcome
}

// editFrame is an edit, as operation, of the user's message target into
// replacement, from the snapshot the editor reads over the socket now: the
// transcript and its version.
func editFrame(t *testing.T, socket *backendtest.Socket, target, operation, replacement string) backendtest.Frame {
	t.Helper()
	socket.Send(backendtest.Frame{"type": "sync_transcript"})
	synced := socket.UntilType("transcript_reset")
	reset := synced[len(synced)-1]
	blocks, _ := reset["blocks"].([]any)
	var targetID any
	for _, block := range blocks {
		if backendtest.At(block, "type") == "user" && backendtest.At(block, "content.0.text") == target && len(backendtest.At(block, "content").([]any)) == 1 {
			targetID = backendtest.At(block, "id")
		}
	}
	if targetID == nil {
		t.Fatalf("the message %q is not in %v", target, backendtest.BlockKinds(blocks))
	}
	return backendtest.Frame{"type": "edit_and_send", "request": backendtest.Map{
		"operationId": operation, "targetBlockId": targetID, "version": reset["version"],
		"content": []any{backendtest.Map{"type": "text", "text": replacement}},
	}}
}

// isIdle reports whether a frame tells the page that the session is idle.
func isIdle(frame backendtest.Frame) bool {
	return backendtest.IsPhase(frame, "idle")
}

// An edit the page sends over the socket replaces its message and what followed
// it, gives the model only the history it kept, restores the todos to the point
// before the edited message and leaves the files the removed turns wrote. Sent
// again over another socket of the conversation, or after a restart of the
// backend, the same edit answers its receipt without asking the model again, and
// an edit from the old snapshot is refused.
//
// Cost: one backend started twice, a scripted vendor and a real runner, a few
// seconds.
func TestAnEditRestoresTheTodosKeepsTheFilesAndAnswersItsReceiptOnAnotherSocketAndAfterARestart(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	paired := onDeviceConversation(t, b, master, convFirst)
	root := paired.Home()
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	socket := b.Connect(master, convFirst)
	socket.Open()
	shell := func(id, script string) *scripted.Response {
		return backendtest.ShellCall(id, script, 60*time.Second)
	}
	vendor.Respond(scripted.Answer([]string{"answer-A-kept"}, 1, 1))
	socket.Chat("m1", "A-kept")
	vendor.Respond(shell("toolu_effects", `printf permanent > sentinel.txt && demi todo add "permanent todo"`))
	vendor.Respond(scripted.Answer([]string{"answer-B-removed"}, 1, 1))
	socket.Chat("m2", "B-removed")
	vendor.Respond(scripted.Answer([]string{"answer-C-removed"}, 1, 1))
	socket.Chat("m3", "C-removed")

	repeated := editFrame(t, socket, "B-removed", "edit-1", "B-edited")
	stale := backendtest.Frame{"type": "edit_and_send", "request": cloneMap(repeated["request"].(backendtest.Map), "operationId", "edit-2")}

	asked := len(vendor.Requests())
	vendor.Respond(scripted.Answer([]string{"answer-edited"}, 1, 1))
	accepted := editOutcome(socket, repeated)
	if accepted["status"] != "accepted" {
		t.Fatalf("the edit is %v", accepted)
	}
	socket.Until(isIdle)
	// The model reads the history the edit kept and the new message, on a fresh
	// runtime; the removed turns and their tool result are gone.
	replayed := jsonText(backendtest.At(vendor.Requests()[asked].JSON(t), "messages"))
	contains(t, replayed, "A-kept", "answer-A-kept", "B-edited")
	for _, gone := range []string{"B-removed", "C-removed", "tool_result"} {
		if containsText(replayed, gone) {
			t.Fatalf("the model still reads %q: %s", gone, replayed)
		}
	}
	// The files the removed turn wrote stay.
	if got := readFile(t, filepath.Join(root, "sentinel.txt")); got != "permanent" {
		t.Fatalf("sentinel.txt holds %q", got)
	}

	// Another socket of the conversation, beside the first: the same edit answers
	// its receipt there, and the old snapshot is refused, neither asking the
	// model.
	second := b.Connect(master, convFirst)
	second.Open()
	backendtest.AssertJSON(t, editOutcome(second, repeated), accepted)
	if outcome := editOutcome(second, stale); outcome["status"] != "rejected" {
		t.Fatalf("the old snapshot's edit is %v", outcome)
	}
	socket.Close()
	second.Close()

	// After a restart as well: the receipt is durable. The backend comes back at
	// its address, where the device's runner reconnects.
	b.Stop()
	b = h.Start()
	b.UntilOnline(master, paired.ID(), true)
	third := b.Connect(master, convFirst)
	third.Open()
	backendtest.AssertJSON(t, editOutcome(third, repeated), accepted)
	if outcome := editOutcome(third, stale); outcome["status"] != "rejected" {
		t.Fatalf("the old snapshot's edit after a restart is %v", outcome)
	}
	if got := len(vendor.Requests()); got != asked+1 {
		t.Fatalf("an edit asked the model again: %d requests, not %d", got, asked+1)
	}

	// The todos are as they were before the edited message; the file stays.
	before := len(vendor.Requests())
	vendor.Respond(shell("toolu_check", "cat sentinel.txt && demi todo list"))
	vendor.Respond(scripted.Answer([]string{"checked"}, 1, 1))
	third.Chat("m4", "Verify the effects")
	checked := scripted.ToolResult(t, vendor.Requests()[before+1].JSON(t), "toolu_check")
	contains(t, checked, "permanent", "No todos")
	if containsText(checked, "permanent todo") {
		t.Fatalf("the removed todo is back: %s", checked)
	}
	b.Stop()
}
