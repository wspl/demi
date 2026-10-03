package database

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
	"go.uber.org/goleak"
)

// These SQLite scenarios cost local temporary files only; no network or wall-time waits.
func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

type testClock struct{ at core.Timestamp }

func (c *testClock) Now() core.Timestamp { return c.at }
func require(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
}

func equal(t *testing.T, want, got any) {
	t.Helper()
	if diff := cmp.Diff(want, got); diff != "" {
		t.Fatalf("(-want +got):\n%s", diff)
	}
}

func testControl(t *testing.T) (*ControlService, *testClock) {
	t.Helper()
	clock := &testClock{at: core.UnixEpoch}
	c, err := OpenControl(t.Context(), filepath.Join(t.TempDir(), "control.sqlite"), clock)
	require(t, err)
	t.Cleanup(func() { require(t, c.Close(context.Background())) })
	return c, clock
}

func testMaster(t *testing.T, c *ControlService) webapi.UserDTO {
	t.Helper()
	hash, err := ParsePasswordHash(
		"$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$0mUbQTTMhhaEBFGMq7WTZxOlVoS9sY3qVqLiV7Q1Izo",
	)
	require(t, err)
	u, err := c.CreateMaster(t.Context(), "master@example.test", hash)
	require(t, err)
	if u == nil {
		t.Fatal("master missing")
	}
	return *u
}

func conversation(n int) webapi.ConversationID {
	return webapi.ConversationID(fmt.Sprintf("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a%02x", n))
}

func createConversation(t *testing.T, c *ControlService, user webapi.UserID, n int) ConversationRecord {
	t.Helper()
	created, isNew, err := c.CreateConversation(t.Context(), user, conversation(n))
	require(t, err)
	if !isNew {
		t.Fatalf("creation %T", created)
	}
	return created
}

func testStores(t *testing.T, limit int) *ConversationStores {
	t.Helper()
	s, err := OpenConversations(t.Context(), t.TempDir(), limit)
	require(t, err)
	t.Cleanup(func() { require(t, s.Close(context.Background())) })
	return s
}

func TestOtherSchemaRefusedAndCurrentReopens(t *testing.T) {
	for _, schema := range []string{
		"CREATE TABLE users(id TEXT PRIMARY KEY); PRAGMA user_version=1;",
		"CREATE TABLE schema_migrations(version INTEGER);",
	} {
		path := filepath.Join(t.TempDir(), "other.sqlite")
		db, err := sql.Open("sqlite", path)
		require(t, err)
		_, err = db.ExecContext(t.Context(), schema)
		require(t, err)
		require(t, db.Close())
		_, err = OpenControl(t.Context(), path, core.SystemClock{})
		var storage *Error
		if !errors.As(err, &storage) || storage.Kind != OtherSchema {
			t.Fatalf("schema refusal: %v", err)
		}
	}
	path := filepath.Join(t.TempDir(), "control.sqlite")
	c, err := OpenControl(t.Context(), path, core.SystemClock{})
	require(t, err)
	testMaster(t, c)
	require(t, c.Close(t.Context()))
	c, err = OpenControl(t.Context(), path, core.SystemClock{})
	require(t, err)
	defer func() { require(t, c.Close(context.Background())) }()
	users, err := c.Users(t.Context())
	require(t, err)
	equal(t, 1, len(users))
}

func TestExpiredWebSessionsSweptOnOpen(t *testing.T) {
	c, clock := testControl(t)
	user := testMaster(t, c)
	policy := SessionPolicy{Lifetime: 30 * 24 * time.Hour, RenewBelow: 15 * 24 * time.Hour}
	for _, token := range []string{"first", "second"} {
		_, err := c.OpenWebSession(t.Context(), HashToken(token), user.ID, policy)
		require(t, err)
	}
	at, err := later(clock.at, policy.Lifetime)
	require(t, err)
	clock.at = at
	_, err = c.OpenWebSession(t.Context(), HashToken("third"), user.ID, policy)
	require(t, err)
	var count int
	require(t, controlDo(t.Context(), c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		return tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM web_sessions").Scan(&count)
	}))
	equal(t, 1, count)
	live, err := c.ResolveWebSession(t.Context(), HashToken("third"), policy)
	require(t, err)
	equal(t, user, live.User)
}

func TestPreferencesAreUserOwnedAndCorruptionRefused(t *testing.T) {
	c, _ := testControl(t)
	user := testMaster(t, c)
	require(t, controlDo(t.Context(), c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		return execSQL(ctx, tx, "INSERT INTO users VALUES ('other-user','other@example.test','','x','user',0)")
	}))
	patch := webapi.PreferencesPatch{Appearance: &webapi.Appearance{Theme: new(webapi.ThemeDark)}}
	saved, err := c.PatchPreferences(t.Context(), user.ID, patch)
	require(t, err)
	equal(t, new(webapi.ThemeDark), saved.Appearance.Theme)
	read, err := c.Preferences(t.Context(), user.ID)
	require(t, err)
	equal(t, saved, read)
	other, err := c.Preferences(t.Context(), "other-user")
	require(t, err)
	equal(t, webapi.Preferences{}, other)
	require(t, controlDo(t.Context(), c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		return execSQL(ctx, tx, `UPDATE user_preferences SET preferences='{"appearance":{}}'`)
	}))
	_, err = c.Preferences(t.Context(), user.ID)
	assertCorrupt(t, err, "user_preferences", "preferences")
}

func assertCorrupt(t *testing.T, err error, table, column string) {
	t.Helper()
	var e *Error
	if !errors.As(err, &e) || e.Kind != Corrupt || e.Table != table || e.Column != column {
		t.Fatalf("corruption %s.%s: %v", table, column, err)
	}
}

func TestConversationSpellingOrderingAndCorruption(t *testing.T) {
	c, _ := testControl(t)
	user := testMaster(t, c)
	first := createConversation(t, c, user.ID, 1)
	second := createConversation(t, c, user.ID, 2)
	equal(t, "New conversation", first.Title)
	equal(t, &webapi.ConversationTargetCloud{}, first.Target)
	if first.Model != nil || first.ReadRevision != 0 {
		t.Fatal("initial metadata")
	}
	upper := webapi.ConversationID(strings.ToUpper(string(first.ID)))
	read, err := c.Conversation(t.Context(), upper)
	require(t, err)
	equal(t, &first, read)
	record, isNew, err := c.CreateConversation(t.Context(), user.ID, upper)
	require(t, err)
	equal(t, false, isNew)
	equal(t, first, record)
	listed, err := c.Conversations(t.Context(), user.ID, false)
	require(t, err)
	equal(t, []ConversationRecord{second, first}, listed)
	archived, err := c.Conversations(t.Context(), user.ID, true)
	require(t, err)
	equal(t, 0, len(archived))
	require(t, c.MarkConversationRead(t.Context(), first.ID, 5))
	require(t, c.MarkConversationRead(t.Context(), first.ID, 3))
	model := storetest.TestModel()
	require(t, c.ChangeConversation(t.Context(), first.ID, &RecordModel{Model: model}))
	require(t, c.TouchConversation(t.Context(), first.ID))
	read, err = c.Conversation(t.Context(), first.ID)
	require(t, err)
	equal(t, uint64(5), read.ReadRevision)
	equal(t, &model, read.Model)
	if read.UpdatedAt < first.UpdatedAt {
		t.Fatal("touch moved updated_at backwards")
	}
	require(t, controlDo(t.Context(), c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		return execSQL(ctx, tx, `UPDATE conversations SET model='{"providerId":""}' WHERE id=?`, first.ID)
	}))
	_, err = c.Conversation(t.Context(), first.ID)
	assertCorrupt(t, err, "conversations", "model")
}

func TestSavedWakeupsListUnarchivedEarliestFirst(t *testing.T) {
	c, _ := testControl(t)
	user := testMaster(t, c)
	for n := 1; n <= 4; n++ {
		createConversation(t, c, user.ID, n)
	}
	at, err := core.TimestampFromMillisecond(9000)
	require(t, err)
	due := &WakeupAt{At: at}
	require(t, c.SetWakeup(t.Context(), conversation(1), due))
	require(t, c.SetWakeup(t.Context(), conversation(2), &WakeupAtStart{}))
	require(t, c.SetWakeup(t.Context(), conversation(3), due))
	err = c.ChangeConversation(t.Context(), conversation(3), &RecordArchived{Archived: true})
	require(t, err)
	require(t, c.SetWakeup(t.Context(), conversation(4), due))
	require(t, c.SetWakeup(t.Context(), conversation(4), nil))
	saved, err := c.SavedWakeups(t.Context())
	require(t, err)
	equal(
		t,
		[]SavedWakeup{
			{Conversation: conversation(2), Owner: user.ID, Due: &WakeupAtStart{}},
			{Conversation: conversation(1), Owner: user.ID, Due: due},
		},
		saved,
	)
}

func TestHostAttachesOnceUnderFreeName(t *testing.T) {
	c, _ := testControl(t)
	user := testMaster(t, c)
	id := createConversation(t, c, user.ID, 1).ID
	devices := make([]DeviceRecord, 0)
	for i, name := range []string{"laptop", "laptop", "ci"} {
		d, err := c.CreateDevice(t.Context(), user.ID, name, runnerwire.RunnerPlatformLinux, HashToken(fmt.Sprint(i)))
		require(t, err)
		devices = append(devices, d)
	}
	attaching := []AttachedHostRecord{
		{Device: devices[0].ID, Name: "laptop"},
		{Device: devices[1].ID, Name: "laptop", CWD: new("/work")},
		{Device: devices[2].ID, Name: " "},
		{Device: devices[0].ID, Name: "renamed", CWD: new("/elsewhere")},
	}
	require(t, controlDo(t.Context(), c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) error {
		for i, h := range attaching {
			changed, err := InsertAttachedHost(ctx, tx, id, h, now)
			if err != nil {
				return err
			}
			equal(t, i < 3, changed)
		}
		return nil
	}))
	expected := []AttachedHostRecord{
		attaching[0],
		{Device: devices[1].ID, Name: "laptop-2", CWD: new("/work")},
		{Device: devices[2].ID, Name: string(devices[2].ID)},
	}
	slices.SortFunc(expected, func(a, b AttachedHostRecord) int { return strings.Compare(a.Name, b.Name) })
	got, err := c.AttachedHosts(t.Context(), id)
	require(t, err)
	equal(t, expected, got)
}

func TestConcurrentTargetSwitchKeepsWinnerAnnouncement(t *testing.T) {
	c, _ := testControl(t)
	user := testMaster(t, c)
	id := createConversation(t, c, user.ID, 1).ID
	laptop, err := c.CreateDevice(t.Context(), user.ID, "laptop", runnerwire.RunnerPlatformLinux, HashToken("laptop"))
	require(t, err)
	type result struct {
		path string
		won  bool
		err  error
	}
	results := make(chan result, 2)
	var wg sync.WaitGroup
	for _, path := range []string{"/first", "/second"} {
		wg.Go(func() {
			target := &webapi.ConversationTargetDevice{DeviceID: laptop.ID, Path: path}
			announcement := TargetSwitch{
				From: &ExecutionCloud{Path: "/home/demi/sessions/" + string(id)},
				To:   &ExecutionDevice{DeviceID: laptop.ID, Path: path},
			}
			won, err := c.SwitchConversationTarget(
				t.Context(),
				id,
				&webapi.ConversationTargetCloud{},
				target,
				announcement,
				SwitchEnds{Arriving: &laptop.ID},
			)
			results <- result{path, won, err}
		})
	}
	wg.Wait()
	close(results)
	winner := ""
	for result := range results {
		require(t, result.err)
		if result.won {
			if winner != "" {
				t.Fatal("two winners")
			}
			winner = result.path
		}
	}
	if winner == "" {
		t.Fatal("no winner")
	}
	record, err := c.Conversation(t.Context(), id)
	require(t, err)
	equal(t, uint64(1), record.ContextVersion)
	equal(t, &webapi.ConversationTargetDevice{DeviceID: laptop.ID, Path: winner}, record.Target)
	announcement, err := c.LastSwitch(t.Context(), id)
	require(t, err)
	equal(t, &ExecutionDevice{DeviceID: laptop.ID, Path: winner}, announcement.To)
}

func writeRoot(ctx context.Context, db *ConversationDB, id webapi.ConversationID, state string) error {
	return db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		return execSQL(
			ctx,
			tx,
			`INSERT INTO nodes (
    id,
    number,
    parent_id,
    description,
    profile,
    round,
    started_at,
    can_spawn,
    delivered,
    state,
    block_count,
    command_revision,
    output_revision
)
VALUES (?,0,NULL,'',NULL,1,0,1,0,?,0,0,0)
ON CONFLICT(id) DO UPDATE
SET state=excluded.state`,
			id,
			state,
		)
	})
}

func rootState(ctx context.Context, tx *sql.Tx) (string, error) {
	var state string
	err := tx.QueryRowContext(ctx, "SELECT state FROM nodes WHERE parent_id IS NULL").Scan(&state)
	return state, err
}

func TestWritersBoundedColdReadsCreateNothing(t *testing.T) {
	stores := testStores(t, 2)
	ctx := t.Context()
	a, b, c := conversation(1), conversation(2), conversation(3)
	first := stores.DB(a)
	require(t, writeRoot(ctx, first, a, "a"))
	require(t, writeRoot(ctx, stores.DB(b), b, "b"))
	equal(t, 2, openWriters(stores))
	found, err := stores.DB(c).Read(ctx, func(context.Context, *sql.Tx) error {
		t.Error("absent read ran")
		return nil
	})
	require(t, err)
	equal(t, false, found)
	_, err = os.Stat(filepath.Join(stores.directory, string(c)+".sqlite"))
	if !errors.Is(err, os.ErrNotExist) {
		t.Fatal(err)
	}
	read := func(db *ConversationDB, want string) {
		t.Helper()
		var got string
		found, err := db.Read(ctx, func(ctx context.Context, tx *sql.Tx) error {
			var err error
			got, err = rootState(ctx, tx)
			return err
		})
		require(t, err)
		equal(t, true, found)
		equal(t, want, got)
	}
	read(stores.DB(b), "b")
	equal(t, 2, openWriters(stores))
	require(t, writeRoot(ctx, stores.DB(c), c, "c"))
	equal(t, 2, openWriters(stores))
	require(t, first.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		got, err := rootState(ctx, tx)
		equal(t, "a", got)
		return err
	}))
	require(t, writeRoot(ctx, first, a, "a2"))
	equal(t, 2, openWriters(stores))
	read(stores.DB(webapi.ConversationID(strings.ToUpper(string(a)))), "a2")
	require(t, stores.Close(ctx))
	equal(t, 0, openWriters(stores))
	if err := first.Call(ctx, func(context.Context, *sql.Tx) error { return nil }); !errors.Is(err, ErrClosed) {
		t.Fatal(err)
	}
	_, err = first.Read(ctx, func(context.Context, *sql.Tx) error { return nil })
	if !errors.Is(err, ErrClosed) {
		t.Fatal(err)
	}
	reopened, err := OpenConversations(ctx, stores.directory, MaxWriters)
	require(t, err)
	defer func() { require(t, reopened.Close(context.Background())) }()
	read(reopened.DB(a), "a2")
	read(reopened.DB(b), "b")
	read(reopened.DB(c), "c")
	equal(t, 0, openWriters(reopened))
}

func TestConcurrentFirstCallsOpenOneWriter(t *testing.T) {
	stores := testStores(t, MaxWriters)
	id := conversation(1)
	results := make(chan error, 2)
	var wg sync.WaitGroup
	for _, state := range []string{"first", "second"} {
		wg.Go(func() { results <- writeRoot(t.Context(), stores.DB(id), id, state) })
	}
	wg.Wait()
	close(results)
	for err := range results {
		require(t, err)
	}
	equal(t, 1, openWriters(stores))
}

type testBlobs struct {
	*storetest.MemoryBlobs
	refuse  error
	touched []core.BlobRef
}

func (b *testBlobs) Media() store.BlobStore { return b.MemoryBlobs }
func (b *testBlobs) CommitUses(_ context.Context, blobs []core.BlobRef) error {
	b.touched = append(b.touched, blobs...)
	return b.refuse
}

func testTree(t *testing.T) (*TreeStore, *ConversationDB, *testBlobs) {
	t.Helper()
	db := testStores(t, 4).DB(conversation(1))
	blobs := &testBlobs{MemoryBlobs: storetest.NewMemoryBlobs()}
	return NewTreeStore(db, blobs, nil), db, blobs
}

func node(id core.NodeID, parent *core.NodeID, number uint64) store.NodeRecord {
	return store.NodeRecord{
		ID:                id,
		Parent:            parent,
		Number:            number,
		Round:             1,
		StartedAt:         core.UnixEpoch,
		CanSpawnSubagents: true,
	}
}

func update(count int, changes ...store.ChangedBlock) store.CheckpointUpdate {
	return store.CheckpointUpdate{
		State: store.CheckpointState{
			Phase:       core.SessionPhaseIdle,
			Queue:       []core.QueuedMessage{},
			AgentInputs: []store.PendingAgentInput{},
			Wakeups:     []store.ScheduledWakeup{},
			CWD:         "/work",
			Model:       storetest.TestModel(),
			Edits:       []store.EditReceipt{},
		},
		ChangedBlocks: changes,
		BlockCount:    count,
	}
}

func user(id string) core.Block {
	return &core.UserBlock{
		BlockID:   core.BlockID(id),
		TurnID:    core.TurnID(id),
		Timestamp: core.UnixEpoch,
		Selection: storetest.TestModel(),
		Content:   storetest.Text("hello"),
	}
}

func reply(id string) core.Block {
	return &core.TextBlock{
		BlockID:   core.BlockID(id),
		Timestamp: core.UnixEpoch,
		Selection: storetest.TestModel(),
		Text:      "hello",
	}
}

func response(id string) core.Block {
	return &core.ResponseBlock{BlockID: core.BlockID(id), Timestamp: core.UnixEpoch, Selection: storetest.TestModel()}
}

func facts(t *testing.T, db *ConversationDB) SummaryFacts {
	t.Helper()
	var facts SummaryFacts
	_, err := db.Read(t.Context(), func(ctx context.Context, tx *sql.Tx) error {
		var err error
		facts, err = Summary(ctx, tx)
		return err
	})
	require(t, err)
	return facts
}

func TestOutputRevisionIgnoresInputAndAdvancesOnRewrite(t *testing.T) {
	tree, db, _ := testTree(t)
	ctx := t.Context()
	found, err := db.Read(ctx, func(context.Context, *sql.Tx) error { return nil })
	require(t, err)
	equal(t, false, found)
	require(t, tree.CreateNode(ctx, node("root", nil, 0), update(0)))
	equal(t, EmptySummary(), facts(t, db))
	root := tree.SessionStore("root")
	require(t, root.Save(ctx, update(1, store.ChangedBlock{Index: 0, Block: user("u1")}), store.CommitGuard{}))
	equal(t, uint64(0), facts(t, db).Revision)
	require(
		t,
		root.Save(
			ctx,
			update(
				3,
				store.ChangedBlock{Index: 1, Block: reply("a1")},
				store.ChangedBlock{Index: 2, Block: response("r1")},
			),
			store.CommitGuard{},
		),
	)
	equal(t, SummaryFacts{Phase: core.SessionPhaseIdle, Revision: 1, Last: new(TerminalResponse)}, facts(t, db))
	require(t, root.Save(ctx, update(1), store.CommitGuard{}))
	equal(t, SummaryFacts{Phase: core.SessionPhaseIdle, Revision: 2}, facts(t, db))
}

func TestRefusedSaveLeavesWholeCheckpoint(t *testing.T) {
	tree, _, _ := testTree(t)
	ctx := t.Context()
	require(t, tree.CreateNode(ctx, node("root", nil, 0), update(0)))
	root := tree.SessionStore("root")
	before, err := root.Load(ctx)
	require(t, err)
	changed := store.InitialCommandState()
	changed.Versions[0].Values["todos.json"] = json.RawMessage("true")
	save := update(1, store.ChangedBlock{Index: 0, Block: user("u1")})
	save.CommandState = &changed
	save.State.Phase = core.SessionPhaseRunning
	err = root.Save(ctx, save, store.CommitGuard{})
	var refusal *store.Error
	if !errors.As(err, &refusal) || refusal.Kind != store.OperationFailed ||
		refusal.Message != "command-state version 0 is immutable" {
		t.Fatalf("immutable version refusal: %v", err)
	}
	after, err := root.Load(ctx)
	require(t, err)
	equal(t, before, after)
	stale, cancel := context.WithCancel(ctx)
	cancel()
	err = root.Save(ctx, update(1, store.ChangedBlock{Index: 0, Block: user("u1")}), store.NewCommitGuard(stale))
	if !errors.Is(err, store.ErrInvalidated) {
		t.Fatal(err)
	}
	after, err = root.Load(ctx)
	require(t, err)
	equal(t, before, after)
}

func TestHistoryIsDepthFirstInSpawnOrder(t *testing.T) {
	tree, db, _ := testTree(t)
	ctx := t.Context()
	require(t, tree.CreateNode(ctx, node("root", nil, 0), update(1, store.ChangedBlock{Index: 0, Block: user("u1")})))
	for _, record := range []store.NodeRecord{
		node("b", new(core.NodeID("root")), 5),
		node("a", new(core.NodeID("root")), 3),
		node("a1", new(core.NodeID("a")), 4),
	} {
		require(
			t,
			tree.CreateNode(
				ctx,
				record,
				update(1, store.ChangedBlock{Index: 0, Block: reply(string(record.ID) + "-text")}),
			),
		)
	}
	var history History
	_, err := db.Read(ctx, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		history, err = ReadHistory(ctx, tx)
		return err
	})
	require(t, err)
	equal(t, []core.Block{user("u1")}, history.Blocks)
	equal(t, 3, len(history.Subagents))
	for i, id := range []core.NodeID{"a", "a1", "b"} {
		equal(t, id, history.Subagents[i].Record.ID)
		equal(t, []core.Block{reply(string(id) + "-text")}, history.Subagents[i].Blocks)
	}
	equal(t, node("a", new(core.NodeID("root")), 3), history.Subagents[0].Record)
}

func TestTreeStoreContract(t *testing.T) {
	storetest.StoreContract(t, func(t *testing.T) store.TreeStore {
		tree, _, _ := testTree(t)
		return tree
	})
}

func blob(value byte) core.BlobRef {
	return core.BlobRef(fmt.Sprintf("%x", sha256.Sum256([]byte{value})))
}

func openWriters(s *ConversationStores) int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return len(s.writers)
}
