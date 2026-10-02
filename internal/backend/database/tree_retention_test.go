package database

import (
	"context"
	"database/sql"
	"fmt"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
)

func shot(id string, images ...byte) core.Block {
	output := make([]core.ToolResultContentBlock, 0, len(images))
	for _, image := range images {
		output = append(output, &core.ToolImage{Source: &core.ToolMediaRef{Ref: blob(image), MediaType: "image/png"}})
	}
	return &core.ToolCallBlock{BlockID: core.BlockID(id), Timestamp: core.UnixEpoch, Selection: storetest.TestModel(), ToolUseID: "toolu_" + id, ToolName: "shell_exec", Input: "{}", Status: "completed", Output: output}
}
func pasted(id string, image byte) core.Block {
	return &core.UserBlock{BlockID: core.BlockID(id), TurnID: core.TurnID(id), Timestamp: core.UnixEpoch, Selection: storetest.TestModel(), Content: []core.UserContentBlock{&core.UserImage{Source: &core.MediaSourceRef{Ref: blob(image), MediaType: "image/png"}}}}
}
func edited(id string, sides ...byte) core.Block {
	edits := make([]core.EditSegment, 0)
	for i := 0; i+1 < len(sides); i++ {
		edits = append(edits, core.EditSegment{Copies: &core.EditCopies{Original: blob(sides[i]), Modified: blob(sides[i+1])}})
	}
	files := []core.EditedFile{{Path: "/work/notes.md", Kind: "modified", Added: 1, Removed: 1, Edits: edits}}
	return &core.ToolCallBlock{BlockID: core.BlockID(id), Timestamp: core.UnixEpoch, Selection: storetest.TestModel(), ToolUseID: "toolu_" + id, ToolName: "shell_exec", Input: "{}", Status: "completed", Output: []core.ToolResultContentBlock{}, View: &core.ShellView{ShellToolView: core.ShellToolView{Status: "exited", ShellID: "shell-1", CommandID: core.CommandID("command-" + id), ExitCode: new(int32(0)), RunningMs: 1, Chunks: []core.OutputChunk{}, Files: &files, FilesTruncated: new(false)}}}
}

// assertIndex checks the durable index against independently read transcript rows.
func assertIndex(t *testing.T, db *ConversationDB) []string {
	t.Helper()
	var held, derived []string
	_, err := db.Read(t.Context(), func(ctx context.Context, tx *sql.Tx) error {
		var err error
		held, err = queryRecords(ctx, tx, "blob_refs", "SELECT * FROM blob_refs ORDER BY node_id,idx,part", func(r *storedRow) string {
			return fmt.Sprintf("%s/%d/%d/%s/%s/%d", r.text("node_id"), r.integer("idx"), r.integer("part"), r.text("blob"), r.text("holder"), r.integer("at"))
		})
		if err != nil {
			return err
		}
		_, err = queryRecords(ctx, tx, "blocks", "SELECT * FROM blocks ORDER BY node_id,idx", func(r *storedRow) struct{} {
			block := storedJSON(r, "block", core.DecodeBlock)
			if r.err != nil {
				return struct{}{}
			}
			for _, row := range BlobRows(block) {
				at, err := row.At.Millisecond()
				require(t, err)
				derived = append(derived, fmt.Sprintf("%s/%d/%d/%s/%s/%d", r.text("node_id"), r.integer("idx"), row.Part, row.Blob, HolderName(row.Holder), at))
			}
			return struct{}{}
		})
		return err
	})
	require(t, err)
	equal(t, derived, held)
	return held
}
func TestEveryBlockWriteKeepsBlobIndexDerived(t *testing.T) {
	tree, db, blobs := testTree(t)
	ctx := t.Context()
	require(t, tree.CreateNode(ctx, node("root", nil, 0), update(2, store.ChangedBlock{Index: 0, Block: pasted("u1", 1)}, store.ChangedBlock{Index: 1, Block: shot("t1", 2, 3)})))
	assertIndex(t, db)
	root := tree.SessionStore("root")
	save := func(count int, changes ...store.ChangedBlock) {
		t.Helper()
		require(t, root.Save(ctx, update(count, changes...), store.CommitGuard{}))
		assertIndex(t, db)
	}
	save(4, store.ChangedBlock{Index: 2, Block: shot("t2", 4)}, store.ChangedBlock{Index: 3, Block: reply("a1")})
	save(3, store.ChangedBlock{Index: 1, Block: reply("a2")}, store.ChangedBlock{Index: 2, Block: shot("t3", 5, 6)})
	save(1, store.ChangedBlock{Index: 0, Block: pasted("u2", 7)})
	save(3, store.ChangedBlock{Index: 1, Block: shot("t4", 8)}, store.ChangedBlock{Index: 2, Block: edited("e1", 10, 11, 12)})
	require(t, tree.CreateNode(ctx, node("child", new(core.NodeID("root")), 2), update(1, store.ChangedBlock{Index: 0, Block: shot("c1", 9)})))
	require(t, tree.DeleteNode(ctx, "child"))
	assertIndex(t, db)
	before := facts(t, db)
	now, err := core.TimestampFromMillisecond(40 * 24 * 60 * 60 * 1000)
	require(t, err)
	require(t, db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		count, err := RetireMedia(ctx, tx, blobs, transcript.Retirement{Now: now, Idle: true})
		equal(t, 1, count)
		return err
	}))
	equal(t, 5, len(assertIndex(t, db)))
	var holders []string
	_, err = db.Read(ctx, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		holders, err = queryRecords(ctx, tx, "blob_refs", "SELECT holder FROM blob_refs ORDER BY idx,part", func(r *storedRow) string { return r.text("holder") })
		return err
	})
	require(t, err)
	equal(t, []string{"message", "edit_copy", "edit_copy", "edit_copy", "edit_copy"}, holders)
	equal(t, before, facts(t, db))
}
func waiting(due ...*core.Timestamp) store.CheckpointUpdate {
	u := update(0)
	for i, at := range due {
		u.State.Wakeups = append(u.State.Wakeups, store.ScheduledWakeup{ID: core.WakeupID(fmt.Sprintf("w%d", i)), DurationMS: 1000, DueAt: at})
	}
	return u
}
func TestEveryCommitReportsEarliestSavedWakeup(t *testing.T) {
	tree, _, _ := testTree(t)
	ctx := t.Context()
	told := make([]WakeupDue, 0)
	tree.saved = func(_ core.NodeID, due WakeupDue) { told = append(told, due) }
	late, err := core.TimestampFromMillisecond(9000)
	require(t, err)
	early, err := core.TimestampFromMillisecond(5000)
	require(t, err)
	require(t, tree.CreateNode(ctx, node("root", nil, 0), waiting(&late, nil)))
	root := tree.SessionStore("root")
	require(t, root.Save(ctx, waiting(&late), store.CommitGuard{}))
	require(t, tree.CreateNode(ctx, node("child", new(core.NodeID("root")), 1), waiting(&early)))
	require(t, root.Save(ctx, waiting(), store.CommitGuard{}))
	require(t, tree.DeleteNode(ctx, "child"))
	running := waiting(&late)
	running.State.Phase = core.SessionPhaseRunning
	require(t, root.Save(ctx, running, store.CommitGuard{}))
	require(t, tree.CreateNode(ctx, node("child", new(core.NodeID("root")), 1), running))
	equal(t, []WakeupDue{&WakeupAtStart{}, &WakeupAt{At: late}, &WakeupAt{At: early}, &WakeupAt{At: early}, nil, nil, &WakeupAt{At: late}}, told)
}
