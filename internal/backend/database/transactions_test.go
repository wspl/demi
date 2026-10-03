package database

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

func TestDraftConflictsRestoreDismissAndRefusals(t *testing.T) {
	c, _ := testControl(t)
	owner := testMaster(t, c)
	id := createConversation(t, c, owner.ID, 1).ID
	ctx := t.Context()
	empty, err := c.Draft(ctx, id)
	require(t, err)
	equal(t, webapi.EmptyConversationDraft(), empty)
	first, err := c.SaveDraft(ctx, id, owner.ID, 0, "first", nil)
	require(t, err)
	second, err := c.SaveDraft(ctx, id, owner.ID, 0, "second", nil)
	require(t, err)
	equal(
		t,
		&webapi.ReplacedDraft{Revision: first.Revision, Text: "first", Files: []webapi.DraftFile{}},
		second.Replaced,
	)
	restored, err := c.ChangeReplacedDraft(ctx, id, webapi.ReplacedActionRestore, first.Revision)
	require(t, err)
	equal(t, "first", restored.Text)
	equal(t, "second", restored.Replaced.Text)
	dismissed, err := c.ChangeReplacedDraft(ctx, id, webapi.ReplacedActionDismiss, second.Revision)
	require(t, err)
	equal(t, (*webapi.ReplacedDraft)(nil), dismissed.Replaced)
	// A dismissal changes the revision but not when the text was written.
	saved, err := c.SaveDraft(ctx, id, owner.ID, restored.Revision, "third", nil)
	require(t, err)
	equal(t, (*webapi.ReplacedDraft)(nil), saved.Replaced)
	_, err = c.ChangeReplacedDraft(ctx, id, webapi.ReplacedActionRestore, second.Revision)
	if !errors.Is(err, ErrDraftChanged) {
		t.Fatalf("stale restore: %v", err)
	}
	upload, err := c.CreateAttachment(ctx, owner.ID, "text/plain", 3, blob(1), new("abc"))
	require(t, err)
	files := []StagedFile{
		&StagedUpload{ID: upload.ID, FileName: "a.txt"},
		&StagedRemote{DeviceID: "laptop", Path: "/work/file"},
	}
	saved, err = c.SaveDraft(ctx, id, owner.ID, saved.Revision, "file", files)
	require(t, err)
	equal(t, 2, len(saved.Files))
	equal(
		t,
		&webapi.DraftFileUpload{
			Ref:       upload.ID,
			FileName:  "a.txt",
			MediaType: "text/plain",
			Sha256:    blob(1),
			Snippet:   new("abc"),
		},
		saved.Files[0],
	)
	_, err = c.SaveDraft(ctx, id, "someone-else", saved.Revision, "bad", files)
	var missing *UploadNotFoundError
	if !errors.As(err, &missing) {
		t.Fatalf("foreign upload: %v", err)
	}
	err = c.ChangeConversation(ctx, id, &RecordArchived{Archived: true})
	require(t, err)
	_, err = c.SaveDraft(ctx, id, owner.ID, saved.Revision, "archived", nil)
	if !errors.Is(err, ErrArchived) {
		t.Fatalf("archived: %v", err)
	}
	read, err := c.Draft(ctx, id)
	require(t, err)
	equal(t, saved, read)
}

func TestPluginWritesCompareRevisionAndRollbackBlobRefusal(t *testing.T) {
	c, _ := testControl(t)
	owner := testMaster(t, c)
	ctx := t.Context()
	_, _, blobs := testTree(t)
	write := ValueWrite{
		User:     owner.ID,
		Plugin:   "notes",
		Key:      "note",
		Document: json.RawMessage(`{"z":"<&>","a":1}`),
		Blobs:    []core.BlobRef{blob(1)},
	}
	result, err := c.WritePluginValue(ctx, write, blobs)
	require(t, err)
	equal(t, uint64(1), result)
	got, _, err := c.PluginValue(ctx, owner.ID, "notes", "note")
	require(t, err)
	equal(t, write.Document, got.Document)
	_, err = c.WritePluginValue(ctx, write, blobs)
	if !errors.Is(err, ErrRevisionConflict) {
		t.Fatalf("conflict: %v", err)
	}
	write.Revision = new(uint64(1))
	write.Document = json.RawMessage(`{"b":2}`)
	write.Blobs = []core.BlobRef{blob(2)}
	refused := errors.New("being deleted")
	blobs.refuse = refused
	_, err = c.WritePluginValue(ctx, write, blobs)
	if !errors.Is(err, refused) {
		t.Fatalf("refusal %v", err)
	}
	after, _, err := c.PluginValue(ctx, owner.ID, "notes", "note")
	require(t, err)
	equal(t, got, after)
	directory := plugin.HostDirectory{Name: "files", Files: []plugin.DirectoryFile{{Path: "a.txt", Blob: blob(3)}}}
	err = c.SetPluginDirectories(ctx, owner.ID, "notes", []plugin.HostDirectory{directory}, blobs)
	if !errors.Is(err, refused) {
		t.Fatal(err)
	}
	dirs, err := c.PluginDirectories(ctx, owner.ID)
	require(t, err)
	equal(t, 0, len(dirs))
	blobs.refuse = nil
	require(t, c.SetPluginDirectories(ctx, owner.ID, "notes", []plugin.HostDirectory{directory}, blobs))
	dirs, err = c.PluginDirectories(ctx, owner.ID)
	require(t, err)
	equal(t, []plugin.HostDirectory{directory}, dirs["notes"])
	refs, err := c.PluginBlobs(ctx, owner.ID)
	require(t, err)
	equal(t, []core.BlobRef{blob(1), blob(3)}, refs)
	require(t, c.RemovePluginValue(ctx, owner.ID, "notes", "note", 1, blobs))
	require(t, c.SetUserPlugin(ctx, owner.ID, "notes", false))
	choices, err := c.UserPlugins(ctx, owner.ID)
	require(t, err)
	equal(t, map[string]bool{"notes": false}, choices)
}

func TestAdmissionCancellationAndCommittedCheckpoint(t *testing.T) {
	tree, db, _ := testTree(t)
	ctx := t.Context()
	require(t, tree.CreateNode(ctx, node("root", nil, 0), update(0)))
	hold := db.stores.HoldCommits()
	t.Cleanup(hold.Release)
	saveCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	done := make(chan error, 1)
	go func() {
		done <- tree.SessionStore("root").Save(
			saveCtx,
			update(1, store.ChangedBlock{Index: 0, Block: reply("a1")}),
			store.CommitGuard{},
		)
	}()
	// Always release and join even if an assertion fails.
	joined := false
	t.Cleanup(func() {
		hold.Release()
		if !joined {
			<-done
		}
	})
	require(t, hold.UntilWaiting(ctx, 1))
	equal(t, uint64(0), facts(t, db).Revision)
	cancel()
	canceled, stop := context.WithCancel(ctx)
	stop()
	if err := db.Call(canceled, func(context.Context, *sql.Tx) error {
		t.Error("canceled admission ran")
		return nil
	}); !errors.Is(err, context.Canceled) {
		t.Fatalf("admission: %v", err)
	}
	hold.Release()
	err := <-done
	joined = true
	require(t, err)
	equal(t, uint64(1), facts(t, db).Revision)
}

func TestCommandOutputsRetentionAndBlobRefusal(t *testing.T) {
	_, db, blobs := testTree(t)
	ctx := t.Context()
	row := CommandOutput{Command: "command-1", Ended: core.UnixEpoch, Output: &OutputStored{Blob: blob(1)}}
	require(t, db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		return InsertCommandOutputs(ctx, tx, blobs, []CommandOutput{row})
	}))
	require(t, db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		return InsertCommandOutputs(
			ctx,
			tx,
			blobs,
			[]CommandOutput{{Command: row.Command, Ended: row.Ended, Output: &OutputNotStored{Reason: "late"}}},
		)
	}))
	after, err := later(core.UnixEpoch, time.Hour)
	require(t, err)
	blobs.refuse = errors.New("being deleted")
	err = db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		_, err := RemoveExpiredOutputs(ctx, tx, blobs, after, after)
		return err
	})
	if !errors.Is(err, blobs.refuse) {
		t.Fatal(err)
	}
	require(t, db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		read, _, err := ReadCommandOutput(ctx, tx, row.Command)
		equal(t, row, read)
		return err
	}))
	blobs.refuse = nil
	require(t, db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		count, err := RemoveExpiredOutputs(ctx, tx, blobs, after, after)
		equal(t, 1, count)
		return err
	}))
	require(t, db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		read, _, err := ReadCommandOutput(ctx, tx, row.Command)
		if err != nil {
			return err
		}
		equal(t, &OutputRemoved{At: after}, read.Output)
		refs, err := References(ctx, tx)
		equal(t, 0, len(refs))
		return err
	}))
}

func TestImmutableCommandVersionsPreserveJSONNumberKinds(t *testing.T) {
	for _, pair := range [][2]string{{"9007199254740992", "9007199254740993"}, {"1", "1.0"}} {
		t.Run(pair[0]+"_"+pair[1], func(t *testing.T) {
			tree, _, _ := testTree(t)
			ctx := t.Context()
			seed := update(0)
			seed.CommandState = &store.CommandStateSnapshot{
				Versions: []store.CommandVersion{
					{Values: map[store.CommandStorageKey]json.RawMessage{"value": json.RawMessage(pair[0])}},
				},
				Boundaries: []store.SessionBoundary{},
			}
			require(t, tree.CreateNode(ctx, node("root", nil, 0), seed))
			change := update(0)
			change.CommandState = &store.CommandStateSnapshot{
				Versions: []store.CommandVersion{
					{Values: map[store.CommandStorageKey]json.RawMessage{"value": json.RawMessage(pair[1])}},
				},
				Boundaries: []store.SessionBoundary{},
			}
			err := tree.SessionStore("root").Save(ctx, change, store.CommitGuard{})
			if err == nil || !strings.HasSuffix(err.Error(), " is immutable") {
				t.Fatalf("immutable number changed: %v", err)
			}
			loaded, _, err := tree.SessionStore("root").Load(ctx)
			require(t, err)
			equal(t, json.RawMessage(pair[0]), loaded.CommandState.Versions[0].Values["value"])
		})
	}
}
