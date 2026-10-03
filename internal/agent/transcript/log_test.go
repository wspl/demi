package transcript_test

import (
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/agent/transcript/transcripttest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestLogPublishesImmutablePatchesAndSaveRows(t *testing.T) {
	ids := transcripttest.NewSequentialIDs("log")
	log := transcript.NewLog(nil, ids, providertest.FixedClock(core.UnixEpoch))
	model := storetest.TestModel()
	if _, changed := log.TakePatches(); changed {
		t.Fatal("empty log has work")
	}
	if _, completed := log.CompleteTailText(); completed || log.EndsWithOpenText() ||
		log.EndsWithInterruption() {
		t.Fatal("empty log has work")
	}
	log.SignThinking("no reasoning")
	log.MarkLatestAbortResumed()
	log.CompleteToolCall("absent", nil, false, nil)
	if _, changed := log.TakePatches(); changed {
		t.Fatal("absent mutation published")
	}
	userID := log.PushUser("turn", model, storetest.Text("hello"), nil)
	log.OpenThinking(model, "a")
	log.AppendThinking(model, "b")
	log.AppendThinking(model, "c")
	log.SignThinking("signed")
	log.AppendThinking(model, "new thought")
	log.PushRedactedThinking(model, "opaque")
	log.AppendText(model, "first")
	savedText := log.Blocks()[4]
	log.AppendText(model, " second")
	log.AppendText(model, " third")
	if savedText.(*core.TextBlock).Text != "first" {
		t.Fatal("old snapshot changed")
	}
	_, completed := log.CompleteTailText()
	if _, again := log.CompleteTailText(); !completed || again || log.EndsWithOpenText() {
		t.Fatal("tail completion")
	}
	batch, _ := log.TakePatches()
	if _, again := log.TakePatches(); batch.Revision != 1 || log.Version().Revision != 1 || again {
		t.Fatal("revision not atomic")
	}
	if !reflect.DeepEqual(batch.Rows.Indices(5), []int{0, 1, 2, 3, 4}) {
		t.Fatal(batch.Rows.Indices(5))
	}
	// Each add is the value at insertion, not the completed block, and consecutive
	// deltas form one patch. No duplicate patch applier is implemented in Go.
	if len(batch.Patches) != 9 {
		t.Fatalf("patch count %d", len(batch.Patches))
	}
	thought := batch.Patches[1].(*framewire.AddPatch).Value.(*core.ThinkingBlock)
	if thought.Text != "a" || thought.Signature != nil {
		t.Fatal("old thinking patch changed")
	}
	if delta := batch.Patches[2].(*framewire.AppendTextPatch); delta.Index != 1 || delta.Delta != "bc" {
		t.Fatal(delta)
	}
	if delta := batch.Patches[7].(*framewire.AppendTextPatch); delta.Index != 4 || delta.Delta != " second third" {
		t.Fatal(delta)
	}
	if !reflect.DeepEqual(batch.Touched, []core.BlockID{userID, "log-3", "log-4", "log-5", "log-6"}) {
		t.Fatal(batch.Touched)
	}
	if !log.HasUserTurn("turn") || log.HasUserTurn("absent") || log.Find(userID) == nil || log.Find("absent") != nil {
		t.Fatal("input lookup")
	}
	log.SignThinking("resigned") // Row 2 changes below the later insertion floor.
	pointBatch, _ := log.TakePatches()
	boundary := log.InsertCompactionBoundary(3, model, "summary", 2)
	log.PushCompactionMarker(model, boundary, 50)
	moved, _ := log.TakePatches()
	pointBatch.Rows.Merge(moved.Rows)
	if got := pointBatch.Rows.Indices(len(log.Blocks())); !reflect.DeepEqual(got, []int{2, 3, 4, 5, 6}) {
		t.Fatal(got)
	}
	if got := pointBatch.Rows.Indices(2); len(got) != 0 {
		t.Fatal(got)
	}
	log.PushAbort(model)
	stopped := log.Blocks()[len(log.Blocks())-1].(*core.AbortBlock)
	log.MarkLatestAbortResumed()
	if stopped.IsResumed || !log.Find(stopped.BlockID).(*core.AbortBlock).IsResumed {
		t.Fatal("stop patch not immutable")
	}
	rewrite := log.ReplaceAll([]core.Block{log.Find(userID)})
	if _, pending := log.TakePatches(); len(rewrite.Patches) != 1 || len(rewrite.Touched) != 0 ||
		len(rewrite.Rows.Indices(1)) != 0 ||
		pending {
		t.Fatal("rewrite did not supersede pending journal")
	}
	if _, ok := rewrite.Patches[0].(*framewire.ReplacePatch); !ok {
		t.Fatal("rewrite not replace")
	}
	restored := transcript.NewLog(log.Blocks(), ids, providertest.FixedClock(core.UnixEpoch))
	if restored.Version().Epoch == log.Version().Epoch || restored.Version().Revision != 0 {
		t.Fatal("restore reused version")
	}
}

func TestLogToolCompletionUsesLatestExecutingCall(t *testing.T) {
	model := storetest.TestModel()
	log := transcript.NewLog(nil, transcripttest.NewSequentialIDs("call"), providertest.FixedClock(core.UnixEpoch))
	log.PushToolCall(model, provider.ToolCall{ToolUseID: "reuse", ToolName: "run", Input: []byte(`null`)})
	log.PushToolCall(model, provider.ToolCall{ToolUseID: "reuse", ToolName: "run", Input: []byte(`"broken {"`)})
	log.AppendText(model, "not forkable yet")
	if _, completed := log.CompleteTailText(); completed {
		t.Fatal("fork crossed executing call")
	}
	pending := log.PendingToolCalls()
	if !reflect.DeepEqual(
		pending,
		[]transcript.PendingCall{
			{ToolUseID: "reuse", ToolName: "run", Input: "{}"},
			{ToolUseID: "reuse", ToolName: "run", Input: "broken {"},
		},
	) {
		t.Fatal(pending)
	}
	before := log.Blocks()
	log.CompleteToolCall("reuse", []core.ToolResultContentBlock{&core.ToolText{Text: "failed"}}, true, nil)
	if before[1].(*core.ToolCallBlock).Status != "executing" {
		t.Fatal("mutated prior call snapshot")
	}
	if got := log.PendingToolCalls(); len(got) != 1 || got[0].Input != "{}" {
		t.Fatal(got)
	}
	log.CompleteToolCall("reuse", []core.ToolResultContentBlock{&core.ToolText{Text: "done"}}, false, nil)
	if _, completed := log.CompleteTailText(); !completed || len(log.PendingToolCalls()) != 0 {
		t.Fatal("completion did not unblock fork")
	}
	blocks := log.Blocks()
	if blocks[0].(*core.ToolCallBlock).Status != "completed" || blocks[1].(*core.ToolCallBlock).Status != "error" {
		t.Fatal("wrong call completed")
	}
}

func TestLogInputsReachReplayInOrder(t *testing.T) {
	model := storetest.TestModel()
	log := transcript.NewLog(nil, transcripttest.NewSequentialIDs("input"), providertest.FixedClock(core.UnixEpoch))
	log.PushContext("turn", model, "execution", "context")
	log.PushSteer("steer", "turn", model, storetest.Text("steer"))
	log.PushWakeup("wake", "turn", model, "new_turn")
	message := core.AgentMessage{
		ID:          "message",
		Sender:      core.Sender{ID: "agent", Number: 1, Description: "reader", Round: 1},
		RecipientID: "root",
		Timestamp:   core.UnixEpoch,
		Content:     "message",
		Event:       &core.MessageEvent{},
	}
	log.PushAgentMessage("turn", model, message)
	log.PushResume("turn", model)
	log.PushError(model, transcript.InterruptedTurnMessage, new(transcript.InterruptedCode), nil)
	if !log.EndsWithInterruption() {
		t.Fatal("interruption not recognized")
	}
	log.PushResponse(model, core.TokenUsage{InputTokens: 10})
	if log.EndsWithInterruption() {
		t.Fatal("response mistaken for interruption")
	}
	replay := transcript.Replay(requestView(t, log.Blocks(), model.Model, store.HeldMedia{}, provider.RequestLimits{}))
	want := []provider.InferenceItem{
		&provider.UserMessage{Content: storetest.SentText("context")},
		&provider.UserSteer{Content: storetest.SentText("steer")},
		&provider.UserMessage{Content: storetest.SentText(transcripttest.WakeupText)},
		&provider.UserSteer{Content: storetest.SentText(transcripttest.AgentMessageEnvelope(message))},
		&provider.UserMessage{Content: storetest.SentText(transcripttest.ResumeText)},
	}
	if !reflect.DeepEqual(replay.Items, want) {
		t.Fatalf("%#v != %#v", replay.Items, want)
	}
	if log.Find("steer") == nil || log.Find("wake") == nil || log.Find("message") == nil {
		t.Fatal("input identities lost")
	}
}

func TestLogEmptyCollectionsEncodeAsArrays(t *testing.T) {
	log := transcript.NewLog(nil, transcript.RandomIDs{}, providertest.FixedClock(core.UnixEpoch))
	if log.Version().Epoch == "" {
		t.Fatal("empty epoch")
	}
	model := storetest.TestModel()
	id := log.PushUser("turn", model, nil, nil)
	if err := core.ValidateBlock(log.Find(id)); err != nil {
		t.Fatal(err)
	}
	log.PushSteer("steer", "turn", model, nil)
	if err := core.ValidateBlock(log.Find("steer")); err != nil {
		t.Fatal(err)
	}
	log.PushToolCall(model, provider.ToolCall{ToolUseID: "call", ToolName: "run", Input: []byte(`{"z":2,"a":1}`)})
	log.CompleteToolCall("call", nil, false, nil)
	for _, block := range log.Blocks() {
		if err := core.ValidateBlock(block); err != nil {
			t.Fatal(err)
		}
	}
	batch := log.ReplaceAll(nil)
	data, err := provider.JSONBody(batch.Patches)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != `[{"op":"replace","value":[]}]` {
		t.Fatal(string(data))
	}
}
