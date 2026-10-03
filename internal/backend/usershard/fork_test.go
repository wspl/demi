package usershard

import (
	"context"
	"slices"
	"testing"

	"github.com/google/go-cmp/cmp"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

type forkBlobs struct{ *storetest.MemoryBlobs }

func (b forkBlobs) Media() store.BlobStore                         { return b.MemoryBlobs }
func (forkBlobs) CommitUses(context.Context, []core.BlobRef) error { return nil }

// Temporary SQLite databases, no vendor, runner or wall-clock wait.
func TestStartupPublishesCommittedForkAndKeepsUncommittedHidden(t *testing.T) {
	ctx := t.Context()
	control := databasetest.Control(ctx, t, core.SystemClock{})
	stores := databasetest.Conversations(ctx, t, 4)
	owner := databasetest.Master(ctx, t, control).ID
	source := webapi.ConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b")
	committed := webapi.ConversationID("7d1c2e3f-4a5b-4c1e-9d2b-0b6f7f3e8f3a")
	uncommitted := webapi.ConversationID("5a4b3c2d-1e0f-4a1b-8c2d-3e4f5a6b7c8d")
	if _, _, err := control.CreateConversation(ctx, owner, source); err != nil {
		t.Fatal(err)
	}
	var attached []database.AttachedHostRecord
	for _, name := range []string{"laptop", "ci"} {
		device, err := control.CreateDevice(ctx, owner, name, runnerwire.RunnerPlatformLinux, database.HashToken(name))
		if err != nil {
			t.Fatal(err)
		}
		cwd := "/" + name
		entry := database.AttachedHostRecord{Device: device.ID, Name: name, CWD: &cwd}
		attached = append(attached, entry)
		if err := control.ChangeConversation(ctx, source, &database.RecordAttach{Host: entry}); err != nil {
			t.Fatal(err)
		}
	}
	at := core.Timestamp("2026-09-24T08:00:00.000Z")
	model := core.ModelSelection{
		ProviderID: "test",
		Model: core.Model{
			ID:            "test",
			ContextWindow: 10000,
			Thinking:      []core.ThinkingCapability{},
		},
	}
	path := "/home/demi/sessions/" + string(source)
	metadata := database.ForkMetadata{
		Title:         "New conversation (Fork)",
		Target:        &webapi.ConversationTargetCloud{Path: &path},
		Model:         &model,
		CreatedAt:     at,
		AttachedHosts: attached,
	}
	for _, id := range []webapi.ConversationID{committed, uncommitted} {
		reserved, err := control.ReserveFork(
			ctx,
			database.ForkOperation{
				ID:       id,
				Owner:    owner,
				Source:   source,
				Block:    "text-1",
				Metadata: metadata,
			},
		)
		if err != nil || reserved == nil {
			t.Fatalf("reserve = %v, %v", reserved, err)
		}
	}
	tree := database.NewTreeStore(stores.DB(committed), forkBlobs{storetest.NewMemoryBlobs()}, nil)
	initial := store.CheckpointUpdate{
		State: store.CheckpointState{
			Phase:       core.SessionPhaseIdle,
			Queue:       []core.QueuedMessage{},
			AgentInputs: []store.PendingAgentInput{},
			Wakeups:     []store.ScheduledWakeup{},
			CWD:         "/work",
			Model:       model,
			Edits:       []store.EditReceipt{},
		},
	}
	if err := tree.CreateNode(ctx, store.RootRecord(core.NodeID(committed), at), initial); err != nil {
		t.Fatal(err)
	}
	if err := control.DeleteDevice(ctx, attached[1].Device); err != nil {
		t.Fatal(err)
	}
	for range 2 {
		if err := RecoverForks(ctx, control, stores); err != nil {
			t.Fatal(err)
		}
	}
	published, err := control.Conversation(ctx, committed)
	if err != nil || published == nil {
		t.Fatalf("published = %v, %v", published, err)
	}
	if published.Title != metadata.Title || published.Pinned || published.Archived || published.CreatedAt != at {
		t.Fatalf("published metadata = %+v", published)
	}
	if diff := cmp.Diff(metadata.Target, published.Target); diff != "" {
		t.Fatal(diff)
	}
	if diff := cmp.Diff(metadata.Model, published.Model); diff != "" {
		t.Fatal(diff)
	}
	kept, err := control.AttachedHosts(ctx, committed)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(attached[:1], kept); diff != "" {
		t.Fatal(diff)
	}
	records, err := control.Conversations(ctx, owner, false)
	if err != nil {
		t.Fatal(err)
	}
	var listed []webapi.ConversationID
	for _, record := range records {
		listed = append(listed, record.ID)
	}
	if !slices.Equal(listed, []webapi.ConversationID{committed, source}) {
		t.Fatalf("sidebar = %v", listed)
	}
	hidden, err := control.Conversation(ctx, uncommitted)
	if err != nil || hidden != nil {
		t.Fatalf("uncommitted = %v, %v", hidden, err)
	}
	pending, err := control.PendingForks(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 1 || pending[0].ID != uncommitted {
		t.Fatalf("pending = %+v", pending)
	}
}
