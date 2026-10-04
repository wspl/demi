package scenarios_test

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

const (
	conversationThird  = "5a4b3c2d-1e0f-4a1b-8c2d-3e4f5a6b7c8d"
	conversationFourth = "9e8d7c6b-5a49-4382-a716-f5e4d3c2b1a0"
)

// conversationTexts returns assistant text boundaries eligible for a fork.
func conversationTexts(blocks []types.Block) []types.BlockID {
	var ids []types.BlockID
	for _, block := range blocks {
		if text, ok := block.(*types.TextBlock); ok {
			ids = append(ids, text.ID())
		}
	}
	return ids
}

// conversationFork sends the user's fork operation at a completed text boundary.
func conversationFork(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	destination string,
	block types.BlockID,
	status int,
) backendtest.Answer {
	t.Helper()
	return conversationRequest(
		ctx,
		t,
		backend,
		session,
		"POST",
		"/api/conversations/"+conversationFirst+"/fork",
		`{"id":"`+destination+`","blockId":"`+string(block)+`"}`,
		status,
	)
}

// TestForkKeepsChosenHistoryWhileSourceRuns checks that forks retain the selected history while their
// source runs.
func TestForkKeepsChosenHistoryWhileSourceRuns(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	wireMust(t, harness.AddUser(ctx, "ana@example.test", "ana-pass-1", webapiproto.RoleUser))
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	path := "/api/conversations/" + conversationFirst
	conversationRequest(ctx, t, backend, &session, "PATCH", path, `{"title":"Build"}`, 200)
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	raised := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "PATCH", path, `{"thinkingEffort":"high"}`, 200),
		webapiproto.DecodeConversationUpdate,
	)
	source, err := backend.Conversation(ctx, t, &session, conversationFirst)
	wireMust(t, err)
	_, err = source.Open(ctx)
	wireMust(t, err)
	vendor.Respond(conversationAnswer(t, []string{"A1"}, 1, 1))
	_, err = source.Chat(ctx, "m1", "U1")
	wireMust(t, err)
	vendor.Respond(conversationAnswer(t, []string{"A2"}, 1, 1))
	_, err = source.Chat(ctx, "m2", "U2")
	wireMust(t, err)
	history, err := source.Live(ctx)
	wireMust(t, err)
	texts := conversationTexts(history)
	conversationEqual(t, len(texts), 2)
	pending := providertest.EventStream(": thinking\n\n")
	pending.Ending = providertest.Open
	vendor.Respond(pending)
	wireMust(t, source.Send(ctx, backendtest.ConversationText("m3", "U3")))
	vendor.Received(ctx, 3)
	forked := conversationDecode(
		t,
		conversationFork(ctx, t, backend, &session, conversationSecond, texts[0], 201),
		webapiproto.DecodeForkAnswer,
	)
	destination := forked.Conversation
	conversationEqual(t, string(destination.ID), conversationSecond)
	conversationEqual(t, destination.Title, "Build (Fork)")
	conversationEqual(t, destination.Pinned, false)
	conversationEqual(t, destination.Archived, false)
	conversationEqual(t, destination.Status, webapiproto.ConversationStatusIdle)
	conversationEqual(t, destination.Model, raised.Conversation.Model)
	directory := "/home/demi/sessions/" + conversationFirst
	conversationEqual(t, conversationJSON(t, destination.Target), `{"kind":"cloud","path":"`+directory+`"}`)
	conversationEqual(t, destination.Cwd, directory)
	conversationEqual(t, conversationTranscript(ctx, t, backend, &session, conversationSecond).Blocks, history[:2])
	listed := conversationDecode(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", "/api/conversations", "", 200),
		webapiproto.DecodeConversations,
	)
	conversationEqual(t, len(listed.Conversations), 2)
	conversationEqual(
		t,
		[]webapiproto.ConversationID{listed.Conversations[0].ID, listed.Conversations[1].ID},
		[]webapiproto.ConversationID{conversationSecond, conversationFirst},
	)
	again := conversationDecode(
		t,
		conversationFork(ctx, t, backend, &session, conversationSecond, texts[0], 200),
		webapiproto.DecodeForkAnswer,
	)
	conversationEqual(t, again, forked)
	conversationRefusal(
		t,
		conversationFork(ctx, t, backend, &session, conversationSecond, texts[1], 409),
		webapiproto.ErrorCodeForkConflict,
	)
	conversationRefusal(
		t,
		conversationFork(ctx, t, backend, &session, conversationFirst, texts[0], 409),
		webapiproto.ErrorCodeIDUnavailable,
	)
	conversationRefusal(
		t,
		conversationFork(ctx, t, backend, &session, conversationThird, history[0].ID(), 400),
		webapiproto.ErrorCodeInvalidForkTarget,
	)
	conversationCreate(ctx, t, backend, &session, conversationThird)
	for _, body := range []string{
		`{"id":"not-a-uuid","blockId":"` + string(texts[0]) + `"}`,
		`{"id":"` + conversationFourth + `"}`,
		`{"id":"` + conversationFourth + `","blockId":"` + string(texts[0]) + `","title":"mine"}`,
	} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, backend, &session, "POST", path+"/fork", body, 400),
			webapiproto.ErrorCodeInvalidBody,
		)
	}
	ana, err := backend.Login(ctx, "ana@example.test", "ana-pass-1")
	wireMust(t, err)
	conversationRefusal(
		t,
		conversationFork(ctx, t, backend, &ana, conversationFourth, texts[0], 404),
		webapiproto.ErrorCodeConversationNotFound,
	)
	conversationRefusal(
		t,
		conversationRequest(
			ctx,
			t,
			backend,
			&ana,
			"POST",
			"/api/conversations",
			`{"id":"`+conversationSecond+`"}`,
			409,
		),
		webapiproto.ErrorCodeIDUnavailable,
	)
	wireMust(t, source.Stop(ctx))
	dest, err := backend.Conversation(ctx, t, &session, conversationSecond)
	wireMust(t, err)
	_, err = dest.Open(ctx)
	wireMust(t, err)
	vendor.Respond(conversationAnswer(t, []string{"A3"}, 1, 1))
	_, err = dest.Chat(ctx, "m4", "U4")
	wireMust(t, err)
	fields, err := contract.Object(vendor.Requests()[3].Body)
	wireMust(t, err)
	messages, err := contract.Decode[[]json.RawMessage](fields["messages"])
	wireMust(t, err)
	conversationEqual(t, len(messages), 3)
	for _, text := range []string{"U1", "A1", "U4"} {
		if !strings.Contains(string(fields["messages"]), text) {
			t.Fatalf("fork replay lacks %s", text)
		}
	}
	if strings.Contains(string(fields["messages"]), "U2") {
		t.Fatal("fork replay includes later history")
	}
	live, err := dest.Live(ctx)
	wireMust(t, err)
	conversationEqual(t, conversationLastText(t, live), "A3")
	conversationEqual(t, conversationSummary(ctx, t, backend, &session, conversationSecond).Title, "Build (Fork)")
	live, err = source.Live(ctx)
	wireMust(t, err)
	conversationEqual(
		t,
		conversationKinds(t, live),
		[]string{"user", "text", "response", "user", "text", "response", "user", "abort"},
	)
}

// TestForkAfterRestartReadsStoredHistory checks that forking after restart reads durable history.
func TestForkAfterRestartReadsStoredHistory(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	settings := conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8").Model
	source, err := backend.Conversation(ctx, t, &session, conversationFirst)
	wireMust(t, err)
	_, err = source.Open(ctx)
	wireMust(t, err)
	vendor.Respond(conversationAnswer(t, []string{"A1"}, 1, 1))
	_, err = source.Chat(ctx, "m1", "U1")
	wireMust(t, err)
	vendor.Respond(conversationAnswer(t, []string{"A2"}, 1, 1))
	_, err = source.Chat(ctx, "m2", "U2")
	wireMust(t, err)
	wireMust(t, source.Close(ctx))
	wireMust(t, backend.Close(ctx))
	backend, err = harness.Start(ctx, t)
	wireMust(t, err)
	session, err = backend.Login(ctx, backendtest.MasterEmail, backendtest.MasterPassword)
	wireMust(t, err)
	stored := conversationTranscript(ctx, t, backend, &session, conversationFirst).Blocks
	text := conversationTexts(stored)[1]
	forked := conversationDecode(
		t,
		conversationFork(ctx, t, backend, &session, conversationSecond, text, 201),
		webapiproto.DecodeForkAnswer,
	)
	conversationEqual(t, forked.Conversation.Title, "U1 (Fork)")
	conversationEqual(t, forked.Conversation.Model, settings)
	conversationEqual(t, conversationTranscript(ctx, t, backend, &session, conversationSecond).Blocks, stored[:5])
	conversationFork(ctx, t, backend, &session, conversationSecond, text, 200)
	conversationEqual(t, conversationTranscript(ctx, t, backend, &session, conversationFirst).Blocks, stored)
}

// conversationCommands returns command identities visible in shell call history.
func conversationCommands(blocks []types.Block) []string {
	var ids []string
	for _, b := range blocks {
		if call, ok := b.(*types.ToolCallBlock); ok {
			if shell, ok := call.View.(*types.ShellView); ok {
				ids = append(ids, string(shell.CommandID))
			}
		}
	}
	return ids
}

// TestForkSharesEditBlobsWithoutWritingObjects checks that forks share edit blobs without writing new
// objects.
// A real runner installs the native file package and records shared edit blobs.
func TestForkSharesEditBlobsWithoutWritingObjects(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	counts := &blobstest.ObjectCounts{}
	harness.Objects = counts
	built, err := backendtest.BuildPackage(ctx, t, "demi-file")
	wireMust(t, err)
	harness.Config.Native, err = built.Catalog(ctx)
	wireMust(t, err)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationOnDevice(ctx, t, harness, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	source := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationShell(t, "toolu_1", `printf 'hello\n' | demi file create notes.txt`, 60000))
	vendor.Respond(conversationAnswer(t, []string{"Written."}, 1, 1))
	_, err = source.Chat(ctx, "m1", "Write the notes")
	wireMust(t, err)
	blocks := conversationTranscript(ctx, t, backend, &session, conversationFirst).Blocks
	files := func(blocks []types.Block) []types.EditedFile {
		call, ok := blocks[1].(*types.ToolCallBlock)
		if !ok {
			t.Fatalf("not a tool call: %T", blocks[1])
		}
		shell, ok := call.View.(*types.ShellView)
		if !ok || shell.Files == nil {
			t.Fatal("command lacks edits")
		}
		return *shell.Files
	}
	edited := files(blocks)
	copies := edited[0].Edits[0].Copies
	if copies == nil {
		t.Fatal("edit lacks copies")
	}
	original := conversationRequest(ctx, t, backend, &session, "GET", "/api/blobs/"+string(copies.Original), "", 200)
	modified := conversationRequest(ctx, t, backend, &session, "GET", "/api/blobs/"+string(copies.Modified), "", 200)
	conversationEqual(t, string(original.Body), "")
	conversationEqual(t, string(modified.Body), "hello\n")
	texts := conversationTexts(blocks)
	before := counts.Tally()
	conversationFork(ctx, t, backend, &session, conversationSecond, texts[len(texts)-1], 201)
	conversationEqual(t, counts.Tally().Since(before).Puts, uint64(0))
	conversationEqual(t, files(conversationTranscript(ctx, t, backend, &session, conversationSecond).Blocks), edited)
}

// TestForkRestoresTodosAtSelectedHistory checks that forks restore todos at the selected history
// boundary.
// A real runner executes the source and both destinations' todo commands.
func TestForkRestoresTodosAtSelectedHistory(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationOnDevice(ctx, t, harness, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	source := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationShell(t, "toolu_add", `demi todo add "first task"`, 60000))
	vendor.Respond(conversationAnswer(t, []string{"Added."}, 1, 1))
	_, err = source.Chat(ctx, "m1", "Plan")
	wireMust(t, err)
	vendor.Respond(conversationShell(t, "toolu_done", "demi todo done T1", 60000))
	vendor.Respond(conversationAnswer(t, []string{"Done."}, 1, 1))
	_, err = source.Chat(ctx, "m2", "Finish it")
	wireMust(t, err)
	live, err := source.Live(ctx)
	wireMust(t, err)
	texts := conversationTexts(live)
	conversationEqual(t, len(texts), 2)
	for i, destination := range []string{conversationSecond, conversationThird} {
		conversationFork(ctx, t, backend, &session, destination, texts[i], 201)
		socket := conversationOpen(ctx, t, backend, &session, destination)
		before := len(vendor.Requests())
		vendor.Respond(conversationShell(t, "toolu_list", "demi todo list --json", 60000))
		vendor.Respond(conversationAnswer(t, []string{"Listed."}, 1, 1))
		_, err := socket.Chat(ctx, "m3", "What is left?")
		wireMust(t, err)
		listed := conversationToolResult(t, vendor.Requests()[before+1], "toolu_list")
		lines := strings.Split(strings.TrimSpace(listed), "\n")
		fields, err := contract.Object([]byte(lines[len(lines)-1]))
		wireMust(t, err)
		todos, err := contract.Decode[[]json.RawMessage](fields["todos"])
		wireMust(t, err)
		todo, err := contract.Object(todos[0])
		wireMust(t, err)
		conversationEqual(t, string(todo["status"]), []string{`"pending"`, `"done"`}[i])
	}
}

// TestForkReadsOnlyCommandsItsHistoryNames checks that forks expose only the commands named in their
// history.
// A real runner reads retained command output and allocates the next identity.
func TestForkReadsOnlyCommandsItsHistoryNames(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, backend, &session, vendor)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationOnDevice(ctx, t, harness, backend, &session, conversationFirst)
	conversationChoose(ctx, t, backend, &session, conversationFirst, provider, "claude-opus-4-8")
	source := conversationOpen(ctx, t, backend, &session, conversationFirst)
	vendor.Respond(conversationShell(t, "toolu_before", "seq 1 3", 60000))
	vendor.Respond(conversationAnswer(t, []string{"Counted."}, 1, 1))
	_, err = source.Chat(ctx, "m1", "Count")
	wireMust(t, err)
	vendor.Respond(conversationShell(t, "toolu_after", "echo later", 60000))
	vendor.Respond(conversationAnswer(t, []string{"Said."}, 1, 1))
	_, err = source.Chat(ctx, "m2", "Say something")
	wireMust(t, err)
	blocks := conversationTranscript(ctx, t, backend, &session, conversationFirst).Blocks
	commands := conversationCommands(blocks)
	conversationEqual(t, commands, []string{"1", "2"})
	conversationFork(ctx, t, backend, &session, conversationSecond, conversationTexts(blocks)[0], 201)
	socket := conversationOpen(ctx, t, backend, &session, conversationSecond)
	before := len(vendor.Requests())
	vendor.Respond(conversationShell(t, "toolu_read", "demi shell output 1 --raw; demi shell output 2", 60000))
	vendor.Respond(conversationAnswer(t, []string{"Read."}, 1, 1))
	_, err = socket.Chat(ctx, "m3", "What did it count?")
	wireMust(t, err)
	read := conversationToolResult(t, vendor.Requests()[before+1], "toolu_read")
	if !strings.Contains(read, "1\n2\n3\ndemi shell output: no command 2 in this conversation") {
		t.Fatalf("wrong output: %s", read)
	}
	conversationEqual(t, toolstest.Field(read, "commandId"), "3")
}
