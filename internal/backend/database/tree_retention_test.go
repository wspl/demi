package database

import (
	"context"
	"database/sql"
	"fmt"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/types"
)

func shot(id string, images ...byte) types.Block {
	output := make([]types.ToolResultContentBlock, 0, len(images))
	for _, image := range images {
		output = append(output, &types.ToolImage{Source: &types.ToolMediaRef{Ref: blob(image), MediaType: "image/png"}})
	}
	return &types.ToolCallBlock{
		BlockID:   types.BlockID(id),
		Timestamp: types.UnixEpoch,
		Selection: storetest.TestModel(),
		ToolUseID: "toolu_" + id,
		ToolName:  "shell_exec",
		Input:     "{}",
		Status:    "completed",
		Output:    output,
	}
}

func pasted(id string, image byte) types.Block {
	return &types.UserBlock{
		BlockID:   types.BlockID(id),
		TurnID:    types.TurnID(id),
		Timestamp: types.UnixEpoch,
		Selection: storetest.TestModel(),
		Content: []types.UserContentBlock{
			&types.UserImage{Source: &types.MediaSourceRef{Ref: blob(image), MediaType: "image/png"}},
		},
	}
}

func edited(id string, sides ...byte) types.Block {
	edits := make([]types.EditSegment, 0)
	for i := 0; i+1 < len(sides); i++ {
		edits = append(
			edits,
			types.EditSegment{Copies: &types.EditCopies{Original: blob(sides[i]), Modified: blob(sides[i+1])}},
		)
	}
	files := []types.EditedFile{{Path: "/work/notes.md", Kind: "modified", Added: 1, Removed: 1, Edits: edits}}
	return &types.ToolCallBlock{
		BlockID:   types.BlockID(id),
		Timestamp: types.UnixEpoch,
		Selection: storetest.TestModel(),
		ToolUseID: "toolu_" + id,
		ToolName:  "shell_exec",
		Input:     "{}",
		Status:    "completed",
		Output:    []types.ToolResultContentBlock{},
		View: &types.ShellView{
			ShellToolView: types.ShellToolView{
				Status:         "exited",
				ShellID:        "shell-1",
				CommandID:      types.CommandID("command-" + id),
				ExitCode:       new(int32(0)),
				RunningMs:      1,
				Chunks:         []types.OutputChunk{},
				Files:          &files,
				FilesTruncated: new(false),
			},
		},
	}
}

// assertIndex checks the durable index against independently read transcript rows.
func assertIndex(t *testing.T, db *ConversationDB) []string {
	t.Helper()
	var held, derived []string
	_, err := db.Read(t.Context(), func(ctx context.Context, tx *sql.Tx) error {
		var err error
		held, err = queryRecords(
			ctx,
			tx,
			"blob_refs",
			"SELECT * FROM blob_refs ORDER BY node_id,idx,part",
			func(r *storedRow) string {
				return fmt.Sprintf(
					"%s/%d/%d/%s/%s/%d",
					r.text("node_id"),
					r.integer("idx"),
					r.integer("part"),
					r.text("blob"),
					r.text("holder"),
					r.integer("at"),
				)
			},
		)
		if err != nil {
			return err
		}
		_, err = queryRecords(
			ctx,
			tx,
			"blocks",
			"SELECT * FROM blocks ORDER BY node_id,idx",
			func(r *storedRow) struct{} {
				block := storedJSON(r, "block", types.DecodeBlock)
				if r.err != nil {
					return struct{}{}
				}
				for _, row := range BlobRows(block) {
					at, err := row.At.Millisecond()
					require(t, err)
					derived = append(
						derived,
						fmt.Sprintf(
							"%s/%d/%d/%s/%s/%d",
							r.text("node_id"),
							r.integer("idx"),
							row.Part,
							row.Blob,
							HolderName(row.Holder),
							at,
						),
					)
				}
				return struct{}{}
			},
		)
		return err
	})
	require(t, err)
	if len(held) == 0 {
		t.Fatal("blob index holds no rows")
	}
	equal(t, derived, held)
	return held
}

func TestEveryBlockWriteKeepsBlobIndexDerived(t *testing.T) {
	tree, db, blobs := testTree(t)
	ctx := t.Context()
	require(
		t,
		tree.CreateNode(
			ctx,
			node("root", nil, 0),
			update(
				2,
				store.ChangedBlock{Index: 0, Block: pasted("u1", 1)},
				store.ChangedBlock{Index: 1, Block: shot("t1", 2, 3)},
			),
		),
	)
	assertIndex(t, db)
	root := tree.Session("root")
	save := func(count int, changes ...store.ChangedBlock) {
		t.Helper()
		require(t, root.Save(ctx, update(count, changes...), store.CommitGuard{}))
		assertIndex(t, db)
	}
	save(4, store.ChangedBlock{Index: 2, Block: shot("t2", 4)}, store.ChangedBlock{Index: 3, Block: reply("a1")})
	save(3, store.ChangedBlock{Index: 1, Block: reply("a2")}, store.ChangedBlock{Index: 2, Block: shot("t3", 5, 6)})
	save(1, store.ChangedBlock{Index: 0, Block: pasted("u2", 7)})
	save(
		3,
		store.ChangedBlock{Index: 1, Block: shot("t4", 8)},
		store.ChangedBlock{Index: 2, Block: edited("e1", 10, 11, 12)},
	)
	require(
		t,
		tree.CreateNode(
			ctx,
			node("child", new(types.NodeID("root")), 2),
			update(1, store.ChangedBlock{Index: 0, Block: shot("c1", 9)}),
		),
	)
	require(t, tree.DeleteNode(ctx, "child"))
	assertIndex(t, db)
	before := facts(t, db)
	now, err := types.TimestampFromMillisecond(40 * 24 * 60 * 60 * 1000)
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
		holders, err = queryRecords(
			ctx,
			tx,
			"blob_refs",
			"SELECT idx,holder FROM blob_refs ORDER BY idx,part",
			func(r *storedRow) string { return fmt.Sprintf("%d/%s", r.integer("idx"), r.text("holder")) },
		)
		return err
	})
	require(t, err)
	equal(t, []string{"0/message", "2/edit_copy", "2/edit_copy", "2/edit_copy", "2/edit_copy"}, holders)
	equal(t, before, facts(t, db))
}

func waiting(due ...*types.Timestamp) store.CheckpointUpdate {
	u := update(0)
	for i, at := range due {
		u.State.Wakeups = append(
			u.State.Wakeups,
			store.ScheduledWakeup{ID: types.WakeupID(fmt.Sprintf("w%d", i)), DurationMS: 1000, DueAt: at},
		)
	}
	return u
}

func TestEveryCommitReportsEarliestSavedWakeup(t *testing.T) {
	tree, _, _ := testTree(t)
	ctx := t.Context()
	told := make([]WakeupDue, 0)
	tree.saved = func(_ types.NodeID, due WakeupDue) { told = append(told, due) }
	late, err := types.TimestampFromMillisecond(9000)
	require(t, err)
	early, err := types.TimestampFromMillisecond(5000)
	require(t, err)
	require(t, tree.CreateNode(ctx, node("root", nil, 0), waiting(&late, nil)))
	root := tree.Session("root")
	require(t, root.Save(ctx, waiting(&late), store.CommitGuard{}))
	require(t, tree.CreateNode(ctx, node("child", new(types.NodeID("root")), 1), waiting(&early)))
	require(t, root.Save(ctx, waiting(), store.CommitGuard{}))
	require(t, tree.DeleteNode(ctx, "child"))
	running := waiting(&late)
	running.State.Phase = types.SessionPhaseRunning
	require(t, root.Save(ctx, running, store.CommitGuard{}))
	require(t, tree.CreateNode(ctx, node("child", new(types.NodeID("root")), 1), running))
	equal(
		t,
		[]WakeupDue{
			&WakeupAtStart{},
			&WakeupAt{At: late},
			&WakeupAt{At: early},
			&WakeupAt{At: early},
			nil,
			nil,
			&WakeupAt{At: late},
		},
		told,
	)
}
