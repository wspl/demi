package storage

import (
	"context"
	"encoding/json/jsontext"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/backend/storage/agentdata"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/webapi"
)

type testClock struct{ at core.Timestamp }

func (c *testClock) Now() core.Timestamp     { return c.at }
func (c *testClock) advance(d time.Duration) { c.at = core.TruncateTimestamp(c.at.Time().Add(d)) }
func must[T any](t *testing.T, v T, err error) T {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
	return v
}
func fixtureHash(t *testing.T) PasswordHash {
	t.Helper()
	h, err := ParsePasswordHash("$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$0mUbQTTMhhaEBFGMq7WTZxOlVoS9sY3qVqLiV7Q1Izo")
	return must(t, h, err)
}
func controlFixture(t *testing.T) (*Control, *testClock, webapi.UserID) {
	t.Helper()
	clock := &testClock{core.TruncateTimestamp(time.UnixMilli(1700000000000))}
	c, err := OpenControl(t.Context(), filepath.Join(t.TempDir(), "control.sqlite"), clock)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := c.Close(); err != nil {
			t.Error(err)
		}
	})
	email, err := webapi.ParseEmailAddress("master@example.test")
	if err != nil {
		t.Fatal(err)
	}
	user, err := c.CreateMaster(t.Context(), email, fixtureHash(t))
	if err != nil || user == nil {
		t.Fatalf("setup: %v %v", user, err)
	}
	return c, clock, user.ID
}
func conversationID(t *testing.T, suffix string) webapi.ConversationID {
	t.Helper()
	id, err := webapi.ParseConversationID("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a" + suffix)
	return must(t, id, err)
}

// Cost: one small SQLite directory; no real clock waits.
func TestControlTransactionsAndDraftConflict(t *testing.T) {
	c, clock, user := controlFixture(t)
	ctx := t.Context()
	email, _ := webapi.ParseEmailAddress("MASTER@example.test")
	duplicate, err := c.CreateUser(ctx, email, fixtureHash(t), webapi.RoleUser)
	if err != nil || duplicate != nil {
		t.Fatalf("case-insensitive duplicate: %v %v", duplicate, err)
	}
	again, err := c.CreateMaster(ctx, email, fixtureHash(t))
	if err != nil || again != nil {
		t.Fatalf("second setup: %v %v", again, err)
	}
	token := HashToken("not a credential")
	policy := SessionPolicy{Lifetime: 30 * 24 * time.Hour, RenewBelow: 15 * 24 * time.Hour}
	expiry, err := c.OpenWebSession(ctx, token, user, policy)
	if err != nil {
		t.Fatal(err)
	}
	clock.advance(15 * 24 * time.Hour)
	session, err := c.ResolveWebSession(ctx, token, policy)
	if err != nil || session == nil || session.Renewed || session.ExpiresAt != expiry {
		t.Fatalf("renewed at exact threshold: %+v %v", session, err)
	}
	clock.advance(time.Millisecond)
	session, err = c.ResolveWebSession(ctx, token, policy)
	if err != nil || session == nil || !session.Renewed {
		t.Fatalf("not renewed below threshold: %+v %v", session, err)
	}
	clock.advance(30 * 24 * time.Hour)
	session, err = c.ResolveWebSession(ctx, token, policy)
	if err != nil || session != nil {
		t.Fatalf("expired session: %+v %v", session, err)
	}
	first, second := conversationID(t, "01"), conversationID(t, "02")
	for _, id := range []webapi.ConversationID{first, second} {
		if _, err = c.CreateConversation(ctx, user, id); err != nil {
			t.Fatal(err)
		}
	}
	upper, _ := webapi.ParseConversationID(strings.ToUpper(first.String()))
	found, err := c.CreateConversation(ctx, user, upper)
	if err != nil || found.Kind != Existing || found.Record.ID != first {
		t.Fatalf("case retry: %+v %v", found, err)
	}
	if err = c.MarkConversationRead(ctx, first, 7); err != nil {
		t.Fatal(err)
	}
	if err = c.MarkConversationRead(ctx, first, 2); err != nil {
		t.Fatal(err)
	}
	record, err := c.Conversation(ctx, first)
	if err != nil || record.ReadRevision != 7 {
		t.Fatalf("read moved backwards: %+v %v", record, err)
	}
	if _, err = c.ReorderConversations(ctx, user, first, &second); err != nil {
		t.Fatal(err)
	}
	clock.advance(time.Hour)
	if err = c.TouchConversation(ctx, second); err != nil {
		t.Fatal(err)
	}
	order, err := c.ConversationOrder(ctx, user)
	if err != nil || !reflect.DeepEqual(order, []webapi.ConversationID{first, second}) {
		t.Fatalf("activity reordered: %v %v", order, err)
	}
	draft, err := c.SaveDraft(ctx, first, user, 0, "first", nil)
	if err != nil {
		t.Fatal(err)
	}
	draft, err = c.SaveDraft(ctx, first, user, 0, "second", nil)
	if err != nil || draft.Replaced == nil || draft.Replaced.Text != "first" {
		t.Fatalf("lost overwritten draft: %+v %v", draft, err)
	}
	draft, err = c.ChangeReplacedDraft(ctx, first, webapi.ReplacedActionRestore, draft.Replaced.Revision)
	if err != nil || draft.Text != "first" || draft.Replaced.Text != "second" {
		t.Fatalf("restore: %+v %v", draft, err)
	}
	_, err = c.ChangeReplacedDraft(ctx, first, webapi.ReplacedActionDismiss, 1)
	var refusal *DraftRefusal
	if !errors.As(err, &refusal) || refusal.Kind != "changed" {
		t.Fatalf("stale dismissal: %v", err)
	}
	if _, err = c.ChangeConversation(ctx, first, ChangeArchived(true)); err != nil {
		t.Fatal(err)
	}
	_, err = c.SaveDraft(ctx, first, user, draft.Revision, "discarded", nil)
	if !errors.As(err, &refusal) || refusal.Kind != "archived" {
		t.Fatalf("archived draft changed: %v", err)
	}
}

// Cost: one small SQLite directory, proving ownership and compare-and-set writes.
func TestDevicesProvidersAndForkPublication(t *testing.T) {
	c, clock, user := controlFixture(t)
	ctx := t.Context()
	d, err := c.CreateDevice(ctx, user, "Work", runnerproto.RunnerPlatformLinux, HashToken("device"))
	if err != nil {
		t.Fatal(err)
	}
	workspace, err := c.CreateWorkspace(ctx, NewWorkspaceID(), user, d.ID, "/work", "Work")
	if err != nil || workspace == nil {
		t.Fatalf("workspace: %v", err)
	}
	cloud, err := c.ManagedDeviceOrCreate(ctx, user)
	if err != nil {
		t.Fatal(err)
	}
	cloud2, err := c.ManagedDeviceOrCreate(ctx, user)
	if err != nil || cloud.ID != cloud2.ID {
		t.Fatalf("duplicate cloud: %v", err)
	}
	provider, _ := webapi.ParseProviderID("entry")
	credential, _ := webapi.ParseCredentialID("account")
	entry, err := c.InsertProvider(ctx, NewProvider{ID: provider, Owner: user, Family: "codex", Kind: webapi.CredentialKindSubscription, Label: "Codex"}, []CredentialWrite{{ID: credential, Label: "fixture", Source: "fixture", Secret: []byte{1, 2, 3}}})
	if err != nil || entry == nil {
		t.Fatalf("provider: %v", err)
	}
	a, err := c.Credential(ctx, provider, credential)
	if err != nil {
		t.Fatal(err)
	}
	won, err := c.ReplaceCredentialSecret(ctx, provider, credential, []byte{4, 5}, a.Version)
	if err != nil || !won {
		t.Fatalf("refresh: %v", err)
	}
	won, err = c.ReplaceCredentialSecret(ctx, provider, credential, []byte{9}, a.Version)
	if err != nil || won {
		t.Fatalf("stale refresh won: %v", err)
	}
	source, dest := conversationID(t, "01"), conversationID(t, "02")
	if _, err = c.CreateConversation(ctx, user, source); err != nil {
		t.Fatal(err)
	}
	block, _ := core.ParseBlockID("assistant")
	op := ForkOperation{ID: dest, Owner: user, Source: source, Block: block, Metadata: ForkMetadata{Title: "Fork", Target: webapi.ConversationTargetWorkspace{WorkspaceID: workspace.ID}, CreatedAt: clock.Now(), AttachedHosts: []AttachedHostRecord{{Device: d.ID, Name: "Work"}}}}
	if _, err = c.ReserveFork(ctx, op); err != nil {
		t.Fatal(err)
	}
	created, err := c.CreateConversation(ctx, user, dest)
	if err != nil || created.Kind != Unavailable {
		t.Fatalf("reserved id reused: %+v %v", created, err)
	}
	record, err := c.PublishFork(ctx, dest)
	if err != nil || record.Title != "Fork" {
		t.Fatalf("publication: %+v %v", record, err)
	}
	pending, err := c.PendingForks(ctx)
	if err != nil || len(pending) != 0 {
		t.Fatalf("published still pending: %v %v", pending, err)
	}
	deleted, err := c.DeleteWorkspace(ctx, user, workspace.ID)
	if err != nil || deleted.Kind != WorkspaceInUse || deleted.Targeting != 1 {
		t.Fatalf("in-use workspace deleted: %+v %v", deleted, err)
	}
}

// Cost: a few local files and transactions; exercises reads after writer eviction.
func TestCheckpointAtomicityReferencesAndRetention(t *testing.T) {
	ctx := t.Context()
	root := t.TempDir()
	clock := &testClock{core.TruncateTimestamp(time.UnixMilli(1700000000000))}
	objects, err := OpenLocalObjects(root)
	if err != nil {
		t.Fatal(err)
	}
	defer objects.Close()
	user, _ := webapi.ParseUserID("owner")
	blobs := NewBlobStores(objects, clock).ForUser(user)
	ref, err := blobs.Put(ctx, []byte("image"))
	if err != nil {
		t.Fatal(err)
	}
	stores, err := OpenConversationStores(filepath.Join(root, "conversations"), 1)
	if err != nil {
		t.Fatal(err)
	}
	defer stores.Close()
	db := stores.DB(conversationID(t, "01"))
	tree := TreeStore{DB: db, Blobs: blobs}
	node, _ := core.ParseNodeID("root")
	blockID, _ := core.ParseBlockID("shot")
	model := core.ModelSelection{ProviderID: "entry", Model: core.Model{ID: "model", Name: "Fixture", Thinking: []core.ThinkingCapability{}}}
	state := CheckpointState{Phase: core.SessionPhaseIdle, Queue: []core.QueuedMessage{}, AgentInputs: []agentdata.PendingAgentInput{}, Wakeups: []agentdata.ScheduledWakeup{}, Cwd: "/work", Model: model, Harness: "fixture", Edits: []agentdata.EditReceipt{}}
	call := core.BlockToolCall{ToolCallBlock: core.ToolCallBlock{ID: blockID, CreatedAt: clock.Now(), Model: model, ToolUseID: "call", ToolName: "shell_exec", Input: "{}", Status: core.ToolCallStatusCompleted, Output: []core.ToolResultContentBlock{core.ToolResultContentBlockImage{Source: core.ToolMediaSourceRef{Ref: ref, MediaType: "image/png"}}}}}
	update := CheckpointUpdate{State: state, ChangedBlocks: []BlockChange{{Index: 0, Block: call}}, BlockCount: 1}
	if err = tree.CreateNode(ctx, NodeRecord{ID: node, Round: 1, StartedAt: clock.Now(), CanSpawnSubagents: true}, update); err != nil {
		t.Fatal(err)
	}
	facts, err := db.Summary(ctx)
	if err != nil || facts.Revision != 1 {
		t.Fatalf("summary: %+v %v", facts, err)
	}
	commands := initialCommandState()
	commands.Versions[0].Values = map[string]jsontext.Value{"key": jsontext.Value(`1`)}
	update.CommandState = &commands
	update.ChangedBlocks = nil
	update.BlockCount = 0
	if err = tree.Save(ctx, node, update, nil); err == nil || !strings.Contains(err.Error(), "immutable") {
		t.Fatalf("immutable version changed: %v", err)
	}
	checkpoint, err := tree.Load(ctx, node)
	if err != nil || len(checkpoint.Transcript) != 1 {
		t.Fatalf("failed save truncated: %+v %v", checkpoint, err)
	}
	if _, err = stores.DB(conversationID(t, "02")).Next(ctx, core.SequenceCommand); err != nil {
		t.Fatal(err)
	}
	checkpoint, err = tree.Load(ctx, node)
	if err != nil || len(checkpoint.Transcript) != 1 {
		t.Fatalf("evicted writer lost data: %v", err)
	}
	clock.advance(31 * 24 * time.Hour)
	changed, err := db.Retire(ctx, blobs, Retirement{Now: clock.Now(), Idle: true})
	if err != nil || changed != 1 {
		t.Fatalf("retire: %d %v", changed, err)
	}
	refs, err := db.References(ctx)
	if err != nil || len(refs) != 0 {
		t.Fatalf("retired reference stays: %v %v", refs, err)
	}
	facts, err = db.Summary(ctx)
	if err != nil || facts.Revision != 1 {
		t.Fatalf("retention marked unread: %+v %v", facts, err)
	}
	deleted, err := blobs.DeleteUnused(ctx, ref, 24*time.Hour)
	if err != nil || deleted {
		t.Fatalf("released blob skipped grace: %v %v", deleted, err)
	}
	clock.advance(24*time.Hour + time.Millisecond)
	deleted, err = blobs.DeleteUnused(ctx, ref, 24*time.Hour)
	if err != nil || !deleted {
		t.Fatalf("unused blob kept: %v %v", deleted, err)
	}
}

// Cost: copies a 168 KiB Rust fixture, then reads it through Go's record API.
func TestRustDirectoryReadsWithoutMigrationOrRepair(t *testing.T) {
	source := "testdata/rust"
	dest := t.TempDir()
	err := filepath.WalkDir(source, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(source, path)
		if err != nil {
			return err
		}
		target := filepath.Join(dest, rel)
		if entry.IsDir() {
			return os.MkdirAll(target, 0755)
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		return os.WriteFile(target, data, 0600)
	})
	if err != nil {
		t.Fatal(err)
	}
	clock := &testClock{core.TruncateTimestamp(time.UnixMilli(1700000000000))}
	c, err := OpenControl(t.Context(), filepath.Join(dest, "control.sqlite"), clock)
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()
	assertRustRows(t, dest, c.db)
	users, err := c.Users(t.Context())
	if err != nil || len(users) != 1 || users[0].Nickname != "Rust fixture" {
		t.Fatalf("Rust users: %+v %v", users, err)
	}
	user := users[0].ID
	preferences, err := c.Preferences(t.Context(), user)
	if err != nil || preferences.Appearance.Theme == nil || *preferences.Appearance.Theme != webapi.ThemeDark {
		t.Fatalf("Rust preferences: %+v %v", preferences, err)
	}
	devices, err := c.PairedDevices(t.Context(), user)
	if err != nil || len(devices) != 1 || devices[0].Name != "Workstation" {
		t.Fatalf("Rust devices: %+v %v", devices, err)
	}
	workspaces, err := c.Workspaces(t.Context(), user)
	if err != nil || len(workspaces) != 1 || workspaces[0].Path != "/home/work" {
		t.Fatalf("Rust workspaces: %+v %v", workspaces, err)
	}
	conversations, err := c.Conversations(t.Context(), user, false)
	if err != nil || len(conversations) != 1 || conversations[0].ReadRevision != 3 {
		t.Fatalf("Rust conversations: %+v %v", conversations, err)
	}
	draft, err := c.Draft(t.Context(), conversations[0].ID)
	if err != nil || draft.Text != "draft" || draft.Replaced == nil || draft.Replaced.Text != "before" {
		t.Fatalf("Rust draft: %+v %v", draft, err)
	}
	hosts, err := c.AttachedHosts(t.Context(), conversations[0].ID)
	if err != nil || len(hosts) != 1 || hosts[0].Name != "Workstation" {
		t.Fatalf("Rust hosts: %+v %v", hosts, err)
	}
	providers, err := c.Providers(t.Context(), user)
	if err != nil || len(providers) != 1 || !reflect.DeepEqual(providers[0].Config, []byte{1, 2, 3}) {
		t.Fatalf("Rust providers: %+v %v", providers, err)
	}
	totals, err := c.UsageTotals(t.Context(), user)
	if err != nil || len(totals) != 1 || totals[0].InputTokens != 100 || totals[0].CacheWriteTokens != 10 {
		t.Fatalf("Rust usage: %+v %v", totals, err)
	}
	upload, _ := webapi.ParseAttachmentID("rust-upload")
	attachment, err := c.Attachment(t.Context(), upload)
	if err != nil || attachment == nil || attachment.SizeBytes != 3 || *attachment.Snippet != "abc" {
		t.Fatalf("Rust attachment: %+v %v", attachment, err)
	}
	stores, err := OpenConversationStores(filepath.Join(dest, "conversations"), 64)
	if err != nil {
		t.Fatal(err)
	}
	defer stores.Close()
	db := stores.DB(conversations[0].ID)
	sequences, err := db.Sequences(t.Context())
	if err != nil || len(sequences) != 2 || sequences[0].Next != 3 || sequences[1].Next != 8 {
		t.Fatalf("Rust sequences: %+v %v", sequences, err)
	}
	command, _ := core.ParseCommandID("command-rust")
	output, err := db.CommandOutput(t.Context(), command)
	if err != nil || output == nil || output.Output.(OutputStored).Missing.Bytes != 7 {
		t.Fatalf("Rust output: %+v %v", output, err)
	}
	history, err := db.History(t.Context())
	if err != nil || len(history.Blocks) != 1 {
		t.Fatalf("Rust history: %+v %v", history, err)
	}
	panel, err := c.Panel(t.Context(), conversations[0].ID)
	if err != nil || panel == nil || panel.Selection != "change" {
		t.Fatalf("Rust panel: %+v %v", panel, err)
	}
	catalog, err := c.CatalogRecord(t.Context(), providers[0].ID)
	if err != nil || catalog == nil || catalog.Key != "digest" {
		t.Fatalf("Rust catalog: %+v %v", catalog, err)
	}
	pending, err := c.PendingForks(t.Context())
	if err != nil || len(pending) != 1 || pending[0].Metadata.Title != "Fork" {
		t.Fatalf("Rust fork: %+v %v", pending, err)
	}
	device, err := webapi.ParseDeviceID("rust-device")
	if err != nil {
		t.Fatal(err)
	}
	operation, err := c.LatestManagedOperation(t.Context(), device)
	if err != nil || operation == nil || operation.ID.String() != "d6d7ad31-b0f6-4b36-9295-751a1fcf56b8" || operation.Phase != webapi.ResetPhaseReady {
		t.Fatalf("Rust operation: %+v %v", operation, err)
	}
	objects, err := OpenLocalObjects(dest)
	if err != nil {
		t.Fatal(err)
	}
	defer objects.Close()
	data, err := NewBlobStores(objects, clock).ForUser(user).Get(context.Background(), attachment.SHA256)
	if err != nil || string(data) != "abc" {
		t.Fatalf("Rust blob: %q %v", data, err)
	}
}

// Cost: one SQLite file; corrupt stored JSON must stop the read, never default it.
func TestCorruptPreferencesStopsRead(t *testing.T) {
	c, _, user := controlFixture(t)
	_, err := c.db.ExecContext(t.Context(), "INSERT INTO user_preferences VALUES (?,?)", user.String(), `{"appearance":{}}`)
	if err != nil {
		t.Fatal(err)
	}
	_, err = c.Preferences(t.Context(), user)
	var corrupt *CorruptError
	if !errors.As(err, &corrupt) || corrupt.Table != "user_preferences" || corrupt.Column != "preferences" {
		t.Fatalf("corruption hidden: %v", err)
	}
}
