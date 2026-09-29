package backendtest_test

import (
	"encoding/json/v2"
	"net/http"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// forkFourth is a destination id no test takes otherwise.
const forkFourth = "9e8d7c6b-5a49-4382-a716-f5e4d3c2b1a0"

func forkBody(destination, block string) backendtest.Map {
	return backendtest.Map{"id": destination, "blockId": block}
}

// textIDs are the ids of the history's assistant texts, in order.
func textIDs(blocks []any) []string {
	var ids []string
	for _, block := range blocks {
		if backendtest.At(block, "type") == "text" {
			ids = append(ids, backendtest.At(block, "id").(string))
		}
	}
	return ids
}

// commandIDs is the command of each shell call of the history, in order.
func commandIDs(blocks []any) []string {
	var ids []string
	for _, block := range blocks {
		if backendtest.At(block, "type") == "tool_call" && backendtest.At(block, "view.kind") == "shell" {
			ids = append(ids, backendtest.At(block, "view.commandId").(string))
		}
	}
	return ids
}

// onDeviceConversation makes the conversation work in the home of a paired
// device's real runner, as a target switch leaves it.
func onDeviceConversation(t *testing.T, b *backendtest.Backend, master *backendtest.Session, id string) *backendtest.Paired {
	t.Helper()
	paired := b.Pair(master, "laptop")
	b.SwitchTo(master, id, paired, paired.Home())
	return paired
}

// Cost: one backend and a scripted vendor, about a second.
func TestAForkKeepsTheHistoryThroughTheChosenTextWhileTheSourceRunsOn(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	ana := b.CreateUser(master, "ana@example.test", "ana-pass-1", "user")
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	sourcePath := "/api/conversations/" + convFirst
	b.Patch(sourcePath, master, backendtest.Map{"title": "Build"})
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	raised := b.Patch(sourcePath, master, backendtest.Map{"thinkingEffort": "high"})
	settings := raised.At("conversation.model")
	source := b.Connect(master, convFirst)
	source.Open()
	vendor.Respond(scripted.Answer([]string{"A1"}, 1, 1))
	source.Chat("m1", "U1")
	vendor.Respond(scripted.Answer([]string{"A2"}, 1, 1))
	source.Chat("m2", "U2")
	history := source.Live()
	texts := textIDs(history)
	if len(texts) != 2 {
		t.Fatalf("two answers: %v", backendtest.BlockKinds(history))
	}
	firstText, secondText := texts[0], texts[1]
	// The source is running its third turn when the Fork arrives.
	vendor.Respond(scripted.EventStream(": thinking\n\n").StayOpen())
	source.Send(backendtest.SendMessage("m3", "U3"))
	vendor.Received(t, 3)

	path := sourcePath + "/fork"
	created := b.Post(path, master, forkBody(convSecond, firstText)).Expect(http.StatusCreated)
	destination := created.At("conversation")
	if backendtest.At(destination, "id") != convSecond || backendtest.At(destination, "title") != "Build (Fork)" ||
		backendtest.At(destination, "pinned") != false || backendtest.At(destination, "archived") != false ||
		backendtest.At(destination, "status") != "idle" {
		t.Fatalf("the destination is %v", destination)
	}
	backendtest.AssertJSON(t, backendtest.At(destination, "model"), settings)
	// A Cloud destination shares the source's directory.
	directory := "/home/demi/sessions/" + convFirst
	backendtest.AssertJSON(t, backendtest.At(destination, "target"), backendtest.Map{"kind": "cloud", "path": directory})
	if backendtest.At(destination, "cwd") != directory {
		t.Fatalf("the destination works in %v", backendtest.At(destination, "cwd"))
	}
	backendtest.AssertJSON(t, b.Transcript(master, convSecond), history[:2])
	if listed := conversationIDs(t, b, master, "/api/conversations"); !slices.Equal(listed, []string{convSecond, convFirst}) {
		t.Fatalf("the destination first: %v", listed)
	}

	// A retry of the attempt finds its destination; the id with another text, or
	// of a conversation no Fork created, is refused.
	again := b.Post(path, master, forkBody(convSecond, firstText)).Expect(http.StatusOK)
	backendtest.AssertJSON(t, again.Value(), created.Value())
	wantRefusal(t, b.Post(path, master, forkBody(convSecond, secondText)), http.StatusConflict, "fork_conflict", "another text")
	wantRefusal(t, b.Post(path, master, forkBody(convFirst, firstText)), http.StatusConflict, "id_unavailable", "the source's id")
	// A block that is no completed assistant text reserves nothing.
	userBlock := backendtest.At(history[0], "id").(string)
	wantRefusal(t, b.Post(path, master, forkBody(convThird, userBlock)), http.StatusBadRequest, "invalid_fork_target", "a user block")
	b.CreateConversation(master, convThird)
	for _, body := range []backendtest.Map{
		{"id": "not-a-uuid", "blockId": firstText},
		{"id": forkFourth},
		{"id": forkFourth, "blockId": firstText, "title": "mine"},
	} {
		wantRefusal(t, b.Post(path, master, body), http.StatusBadRequest, "invalid_body", jsonText(body))
	}
	// Another user reaches neither the source nor the destination's id.
	wantRefusal(t, b.Post(path, ana, forkBody(forkFourth, firstText)), http.StatusNotFound, "conversation_not_found", "another user's fork")
	wantRefusal(t, b.Post("/api/conversations", ana, backendtest.Map{"id": convSecond}), http.StatusConflict, "id_unavailable", "another user's claim")

	// The source runs on; the destination goes its own way, replaying only the
	// history it kept.
	source.Stop()
	destinationSocket := b.Connect(master, convSecond)
	destinationSocket.Open()
	vendor.Respond(scripted.Answer([]string{"A3"}, 1, 1))
	destinationSocket.Chat("m4", "U4")
	replayed := scenarioItem(t, vendor.Requests(), 3).JSON(t)
	messages := jsonText(backendtest.At(replayed, "messages"))
	if got := len(backendtest.At(replayed, "messages").([]any)); got != 3 {
		t.Fatalf("the destination replays %d messages: %s", got, messages)
	}
	contains(t, messages, "U1", "A1", "U4")
	if strings.Contains(messages, "U2") {
		t.Fatalf("the destination replays what it did not keep: %s", messages)
	}
	if text := lastText(t, destinationSocket.Live()); text != "A3" {
		t.Fatalf("the answer is %q", text)
	}
	// A Fork's title is the user's: the destination's first message leaves it.
	var titles []string
	summaries, _ := b.Get("/api/conversations", master).At("conversations").([]any)
	for _, summary := range summaries {
		titles = append(titles, backendtest.At(summary, "title").(string))
	}
	if !slices.Contains(titles, "Build (Fork)") {
		t.Fatalf("the titles are %v", titles)
	}
	want := []string{"user", "text", "response", "user", "text", "response", "user", "abort"}
	if kinds := backendtest.BlockKinds(source.Live()); !slices.Equal(kinds, want) {
		t.Fatalf("the source's transcript is %v", kinds)
	}
	b.Stop()
}

// Cost: one backend started twice and a scripted vendor, about two seconds.
func TestAForkOfAConversationTheBackendNoLongerHoldsReadsItsStoredHistory(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t)
	b, master := h.StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	settings := b.Choose(master, convFirst, provider, "claude-opus-4-8").At("conversation.model")
	source := b.Connect(master, convFirst)
	source.Open()
	vendor.Respond(scripted.Answer([]string{"A1"}, 1, 1))
	source.Chat("m1", "U1")
	vendor.Respond(scripted.Answer([]string{"A2"}, 1, 1))
	source.Chat("m2", "U2")
	b.Stop()

	b = h.Start()
	master = b.Login(backendtest.MasterEmail, backendtest.MasterPassword)
	stored := b.Transcript(master, convFirst)
	secondText := scenarioItem(t, textIDs(stored), 1)
	path := "/api/conversations/" + convFirst + "/fork"
	created := b.Post(path, master, forkBody(convSecond, secondText)).Expect(http.StatusCreated)
	// The first message titled the source.
	if created.Str("conversation.title") != "U1 (Fork)" {
		t.Fatalf("the destination is titled %q", created.Str("conversation.title"))
	}
	backendtest.AssertJSON(t, created.At("conversation.model"), settings)
	// The history through the latest text, without the response after it.
	if len(stored) < 5 {
		t.Fatalf("the source has fewer than five blocks: %v", stored)
	}
	backendtest.AssertJSON(t, b.Transcript(master, convSecond), stored[:5])
	b.Post(path, master, forkBody(convSecond, secondText)).Expect(http.StatusOK)
	backendtest.AssertJSON(t, b.Transcript(master, convFirst), stored)
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, a few seconds: three
// turns each run a demi todo job on a real device.
func TestAForkKeepsTheTodosItsHistoryHad(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	onDeviceConversation(t, b, master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	source := b.Connect(master, convFirst)
	source.Open()
	shell := func(id, script string) *scripted.Response {
		return backendtest.ShellCall(id, script, 60*time.Second)
	}
	vendor.Respond(shell("toolu_add", `demi todo add "first task"`))
	vendor.Respond(scripted.Answer([]string{"Added."}, 1, 1))
	source.Chat("m1", "Plan")
	vendor.Respond(shell("toolu_done", "demi todo done T1"))
	vendor.Respond(scripted.Answer([]string{"Done."}, 1, 1))
	source.Chat("m2", "Finish it")
	texts := textIDs(source.Live())
	if len(texts) != 2 {
		t.Fatalf("two answers: %v", texts)
	}

	// A Fork's todos are the ones its history had, however the source went on.
	for _, fork := range []struct{ destination, text, status string }{
		{convSecond, texts[0], "pending"}, {convThird, texts[1], "done"},
	} {
		b.Post("/api/conversations/"+convFirst+"/fork", master, forkBody(fork.destination, fork.text)).Expect(http.StatusCreated)
		socket := b.Connect(master, fork.destination)
		socket.Open()
		before := len(vendor.Requests())
		vendor.Respond(shell("toolu_list", "demi todo list --json"))
		vendor.Respond(scripted.Answer([]string{"Listed."}, 1, 1))
		socket.Chat("m3", "What is left?")
		listed := scripted.ToolResult(t, scenarioItem(t, vendor.Requests(), before+1).JSON(t), "toolu_list")
		lines := strings.Split(strings.TrimRight(listed, "\n"), "\n")
		var todos any
		if err := json.Unmarshal([]byte(lines[len(lines)-1]), &todos); err != nil {
			t.Fatalf("the list is not JSON: %v\n%s", err, listed)
		}
		if got := backendtest.At(todos, "todos.0.status"); got != fork.status {
			t.Fatalf("the fork's todo is %v, not %s: %s", got, fork.status, listed)
		}
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real runner, a few seconds: four
// turns run a shell job each on a real device.
func TestAForkReadsTheOutputsOfTheCommandsItsHistoryNames(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	onDeviceConversation(t, b, master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	source := b.Connect(master, convFirst)
	source.Open()
	shell := func(id, script string) *scripted.Response {
		return backendtest.ShellCall(id, script, 60*time.Second)
	}
	vendor.Respond(shell("toolu_before", "seq 1 3"))
	vendor.Respond(scripted.Answer([]string{"Counted."}, 1, 1))
	source.Chat("m1", "Count")
	vendor.Respond(shell("toolu_after", "echo later"))
	vendor.Respond(scripted.Answer([]string{"Said."}, 1, 1))
	source.Chat("m2", "Say something")
	blocks := b.Transcript(master, convFirst)
	commands := commandIDs(blocks)
	if len(commands) != 2 || commands[0] != "1" || commands[1] != "2" {
		t.Fatalf("two commands: %v", commands)
	}
	before, after := commands[0], commands[1]
	counted := scenarioItem(t, textIDs(blocks), 0)

	// The destination's history names the first command, whose output demi shell
	// output reads there as in the source; the second is not the destination's.
	b.Post("/api/conversations/"+convFirst+"/fork", master, forkBody(convSecond, counted)).Expect(http.StatusCreated)
	socket := b.Connect(master, convSecond)
	socket.Open()
	requests := len(vendor.Requests())
	vendor.Respond(shell("toolu_read", "demi shell output "+before+" --raw; demi shell output "+after))
	vendor.Respond(scripted.Answer([]string{"Read."}, 1, 1))
	socket.Chat("m3", "What did it count?")
	read := scripted.ToolResult(t, scenarioItem(t, vendor.Requests(), requests+1).JSON(t), "toolu_read")
	contains(t, read, "1\n2\n3\ndemi shell output: no command "+after+" in this conversation")
	// The destination goes on from its source's numbers.
	if id := backendtest.Field(t, read, "commandId"); id != "3" {
		t.Fatalf("the destination's next command is %s", id)
	}
	b.Stop()
}

// Cost: one backend, a scripted vendor and a real device, a few seconds: the
// device installs the builtin package for the command whose edits the Fork keeps.
func TestAForkReadsTheEditsItsHistoryMadeFromTheSameBlobsAndWritesNoObject(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin()).StartSetUp()
	provider := b.Anthropic(master, vendor, "")
	b.CreateConversation(master, convFirst)
	// The runner lives as long as its device binding.
	onDeviceConversation(t, b, master, convFirst)
	b.Choose(master, convFirst, provider, "claude-opus-4-8")
	source := b.Connect(master, convFirst)
	source.Open()
	vendor.Respond(backendtest.ShellCall("toolu_1", "printf 'hello\\n' | demi file create notes.txt", time.Minute))
	vendor.Respond(scripted.Answer([]string{"Written."}, 1, 1))
	source.Chat("m1", "Write the notes")
	edited := keptFiles(b, master, convFirst)
	if len(edited) != 1 || len(edited[0]) == 0 {
		t.Fatalf("the command lists no change: %v", edited)
	}
	if original, modified := editSides(t, b, master, edited[0][0]); original != "" || modified != "hello\n" {
		t.Fatalf("the edit reads %q and %q", original, modified)
	}

	texts := textIDs(b.Transcript(master, convFirst))
	before := b.Control.ObjectCounts()
	b.Post("/api/conversations/"+convFirst+"/fork", master, forkBody(convSecond, scenarioItem(t, texts, len(texts)-1))).Expect(http.StatusCreated)
	// The destination's call names the same blobs, and the Fork put none.
	if puts := b.Control.ObjectCounts().Since(before).Puts; puts != 0 {
		t.Fatalf("the fork put %d objects", puts)
	}
	backendtest.AssertJSON(t, keptFiles(b, master, convSecond), edited)
	b.Stop()
}
