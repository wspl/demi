package backend_test

import (
	"context"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider/providertest"
)

// conversationEditRequest uses the exact snapshot and version the editor reads.
func conversationEditRequest(ctx context.Context, t *testing.T, s *backendtest.ConversationSocket, target, operation, replacement string) framewire.EditRequest {
	t.Helper()
	wireMust(t, s.Send(ctx, &framewire.SyncTranscriptFrame{}))
	frames, err := s.Until(ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.TranscriptResetFrame)
		return ok
	})
	wireMust(t, err)
	reset := frames[len(frames)-1].(*framewire.TranscriptResetFrame)
	for _, b := range reset.Blocks {
		if user, ok := b.(*core.UserBlock); ok && len(user.Content) == 1 {
			if text, ok := user.Content[0].(*core.UserText); ok && text.Text == target {
				return framewire.EditRequest{OperationID: core.OperationID(operation), TargetBlockID: b.ID(), Version: reset.Version, Content: backendtest.ConversationText("unused", replacement).Content}
			}
		}
	}
	t.Fatalf("no user message %q", target)
	return framewire.EditRequest{}
}

// conversationEdit waits for the durable receipt of a page's edit.
func conversationEdit(ctx context.Context, t *testing.T, s *backendtest.ConversationSocket, request framewire.EditRequest) framewire.EditOutcome {
	t.Helper()
	wireMust(t, s.Send(ctx, &framewire.EditAndSendFrame{Request: request}))
	frames, err := s.Until(ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.EditResultFrame)
		return ok
	})
	wireMust(t, err)
	return frames[len(frames)-1].(*framewire.EditResultFrame).Outcome
}

// conversationIdle identifies the phase a page relies on before editing.
func conversationIdle(f framewire.ServerFrame) bool {
	p, ok := f.(*framewire.PhaseFrame)
	return ok && p.Phase == core.SessionPhaseIdle
}

func TestEditPublishedOnlyAfterCommit(t *testing.T) {
	ctx, h := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, b, &s, vendor)
	conversationCreate(ctx, t, b, &s, conversationFirst)
	conversationChoose(ctx, t, b, &s, conversationFirst, provider, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, b, &s, conversationFirst)
	for i, words := range []string{"A-kept", "B-removed"} {
		vendor.Respond(conversationAnswer(t, []string{"answer-" + words}, 1, 1))
		_, err := socket.Chat(ctx, "m"+string(rune('1'+i)), words)
		wireMust(t, err)
	}
	history := conversationTranscript(ctx, t, b, &s, conversationFirst).Blocks
	request := conversationEditRequest(ctx, t, socket, "B-removed", "edit-1", "B-edited")
	hold := backendtest.HoldCommits(t, b.Backend)
	asked := len(vendor.Requests())
	vendor.Respond(conversationAnswer(t, []string{"answer-edited"}, 1, 1))
	wireMust(t, socket.Send(ctx, &framewire.EditAndSendFrame{Request: request}))
	wireMust(t, hold.UntilWaiting(ctx, 1))
	conversationEqual(t, conversationTranscript(ctx, t, b, &s, conversationFirst).Blocks, history)
	conversationEqual(t, len(vendor.Requests()), asked)
	hold.Release()
	frames, err := socket.Until(ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.EditResultFrame)
		return ok
	})
	wireMust(t, err)
	outcome := frames[len(frames)-1].(*framewire.EditResultFrame).Outcome
	if _, ok := outcome.(*framewire.AcceptedEdit); !ok {
		t.Fatalf("edit refused: %v", outcome)
	}
	if !slices.Contains(conversationKinds(t, frames), "transcript_patch") {
		t.Fatal("edit receipt preceded replacement")
	}
	_, err = socket.Until(ctx, conversationIdle)
	wireMust(t, err)
	replayed := string(vendor.Requests()[asked].Body)
	if !strings.Contains(replayed, "B-edited") || strings.Contains(replayed, "B-removed") {
		t.Fatalf("wrong replay: %s", replayed)
	}
}

func TestTurnEndWaitsForCommitAndImmediatelyAdmitsEdit(t *testing.T) {
	ctx, h := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, b, &s, vendor)
	conversationCreate(ctx, t, b, &s, conversationFirst)
	conversationChoose(ctx, t, b, &s, conversationFirst, provider, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, b, &s, conversationFirst)
	vendor.Respond(conversationAnswer(t, []string{"answer-A"}, 1, 1))
	_, err = socket.Chat(ctx, "m1", "A-kept")
	wireMust(t, err)
	hold := backendtest.HoldCommits(t, b.Backend)
	vendor.Respond(conversationAnswer(t, []string{"answer-B"}, 1, 1))
	wireMust(t, socket.Send(ctx, backendtest.ConversationText("m2", "B-removed")))
	wireMust(t, hold.UntilWaiting(ctx, 1))
	wireMust(t, socket.Send(ctx, &framewire.SyncTranscriptFrame{}))
	frames, err := socket.Until(ctx, func(f framewire.ServerFrame) bool {
		_, ok := f.(*framewire.TranscriptResetFrame)
		return ok
	})
	wireMust(t, err)
	if slices.ContainsFunc(frames, conversationIdle) {
		t.Fatal("page saw idle before commit")
	}
	reset := frames[len(frames)-1].(*framewire.TranscriptResetFrame)
	var target core.BlockID
	for _, b := range reset.Blocks {
		if u, ok := b.(*core.UserBlock); ok && u.TurnID == "m2" {
			target = b.ID()
		}
	}
	if target == "" {
		t.Fatal("reset lacks message")
	}
	request := framewire.EditRequest{OperationID: "edit-1", TargetBlockID: target, Version: reset.Version, Content: backendtest.ConversationText("unused", "B-edited").Content}
	hold.Release()
	_, err = socket.Until(ctx, conversationIdle)
	wireMust(t, err)
	vendor.Respond(conversationAnswer(t, []string{"answer-edited"}, 1, 1))
	outcome := conversationEdit(ctx, t, socket, request)
	if _, ok := outcome.(*framewire.AcceptedEdit); !ok {
		t.Fatalf("edit refused: %v", outcome)
	}
	_, err = socket.Until(ctx, conversationIdle)
	wireMust(t, err)
}

// A real runner executes tool effects and reconnects after the backend restart.
func TestEditRestoresTodosKeepsFilesAndDurableReceipt(t *testing.T) {
	ctx, h := conversationHarness(t)
	vendor := providertest.StartVendor(t)
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	provider := conversationAnthropic(ctx, t, b, &s, vendor)
	conversationCreate(ctx, t, b, &s, conversationFirst)
	paired, root := conversationOnDevice(ctx, t, h, b, &s, conversationFirst)
	conversationChoose(ctx, t, b, &s, conversationFirst, provider, "claude-opus-4-8")
	socket := conversationOpen(ctx, t, b, &s, conversationFirst)
	vendor.Respond(conversationAnswer(t, []string{"answer-A-kept"}, 1, 1))
	_, err = socket.Chat(ctx, "m1", "A-kept")
	wireMust(t, err)
	vendor.Respond(conversationShell(t, "toolu_effects", `printf permanent > sentinel.txt && demi todo add "permanent todo"`, 60000))
	vendor.Respond(conversationAnswer(t, []string{"answer-B-removed"}, 1, 1))
	_, err = socket.Chat(ctx, "m2", "B-removed")
	wireMust(t, err)
	vendor.Respond(conversationAnswer(t, []string{"answer-C-removed"}, 1, 1))
	_, err = socket.Chat(ctx, "m3", "C-removed")
	wireMust(t, err)
	repeated := conversationEditRequest(ctx, t, socket, "B-removed", "edit-1", "B-edited")
	stale := repeated
	stale.OperationID = "edit-2"
	asked := len(vendor.Requests())
	vendor.Respond(conversationAnswer(t, []string{"answer-edited"}, 1, 1))
	accepted := conversationEdit(ctx, t, socket, repeated)
	if _, ok := accepted.(*framewire.AcceptedEdit); !ok {
		t.Fatalf("edit refused: %v", accepted)
	}
	_, err = socket.Until(ctx, conversationIdle)
	wireMust(t, err)
	fields, err := contract.Object(vendor.Requests()[asked].Body)
	wireMust(t, err)
	replayed := string(fields["messages"])
	for _, text := range []string{"A-kept", "answer-A-kept", "B-edited"} {
		if !strings.Contains(replayed, text) {
			t.Fatalf("replay lacks %s", text)
		}
	}
	for _, text := range []string{"B-removed", "C-removed", "tool_result"} {
		if strings.Contains(replayed, text) {
			t.Fatalf("replay kept %s", text)
		}
	}
	file, err := os.ReadFile(filepath.Join(root, "sentinel.txt"))
	wireMust(t, err)
	conversationEqual(t, string(file), "permanent")
	second := conversationOpen(ctx, t, b, &s, conversationFirst)
	conversationEqual(t, conversationEdit(ctx, t, second, repeated), accepted)
	if _, ok := conversationEdit(ctx, t, second, stale).(*framewire.RejectedEdit); !ok {
		t.Fatal("stale edit accepted")
	}
	wireMust(t, socket.Close(ctx))
	wireMust(t, second.Close(ctx))
	address := b.Address()
	wireMust(t, b.Close(ctx))
	b, err = h.StartAt(ctx, t, address)
	wireMust(t, err)
	wireMust(t, b.UntilOnline(ctx, &s, paired.ID(), true))
	third := conversationOpen(ctx, t, b, &s, conversationFirst)
	conversationEqual(t, conversationEdit(ctx, t, third, repeated), accepted)
	if _, ok := conversationEdit(ctx, t, third, stale).(*framewire.RejectedEdit); !ok {
		t.Fatal("stale edit accepted after restart")
	}
	conversationEqual(t, len(vendor.Requests()), asked+1)
	before := len(vendor.Requests())
	vendor.Respond(conversationShell(t, "toolu_check", "cat sentinel.txt && demi todo list", 60000))
	vendor.Respond(conversationAnswer(t, []string{"checked"}, 1, 1))
	_, err = third.Chat(ctx, "m4", "Verify the effects")
	wireMust(t, err)
	checked := conversationToolResult(t, vendor.Requests()[before+1], "toolu_check")
	if !strings.Contains(checked, "permanent") || !strings.Contains(checked, "No todos") || strings.Contains(checked, "permanent todo") {
		t.Fatalf("wrong restored effects: %s", checked)
	}
}
