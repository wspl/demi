package storetest

import (
	"context"
	"crypto/sha256"
	"fmt"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// StoreContract runs the eight shared tree-store scenarios. Each subtest gets
// a fresh, empty store from newStore; the factory registers resource cleanup
// on that subtest. Cases use t.Context and wait for events, never wall time.
func StoreContract(t *testing.T, newStore func(t *testing.T) store.TreeStore) {
	cases := []struct {
		name string
		run  func(context.Context, contractTree)
	}{
		{"create_queues_the_first_message_with_the_node_and_a_save_replaces_it", func(ctx context.Context, c contractTree) {
			c.create(ctx, contractRecord("root", nil, 0), contractUpdate(nil, nil))
			c.create(ctx, contractRecord("child", new(core.NodeID("root")), 1), contractUpdate([]core.QueuedMessage{contractMessage("m1")}, nil))
			children, err := c.store.Children(ctx, "root")
			if err != nil || len(children) != 1 || children[0].ID != "child" {
				c.t.Fatalf("children: %v, %v", children, err)
			}
			if got := c.load(ctx, "child").State.Queue; !reflect.DeepEqual(got, []core.QueuedMessage{contractMessage("m1")}) {
				c.t.Fatalf("queue: %#v", got)
			}
			user := &core.UserBlock{BlockID: "u1", TurnID: "m1", Timestamp: contractEpoch, Selection: TestModel(), Content: Text("brief")}
			c.save(ctx, "child", contractUpdate(nil, []core.Block{user}))
			loaded := c.load(ctx, "child")
			if len(loaded.State.Queue) != 0 || !reflect.DeepEqual(loaded.Transcript, []core.Block{user}) {
				c.t.Fatalf("checkpoint: %#v", loaded)
			}
			if err := c.store.CreateNode(ctx, contractRecord("child", new(core.NodeID("root")), 2), contractUpdate(nil, nil)); err == nil {
				c.t.Fatal("existing node accepted")
			}
		}},
		{"a_save_delivers_the_completions_its_transcript_carries_and_only_those", func(ctx context.Context, c contractTree) {
			c.create(ctx, contractRecord("root", nil, 0), contractUpdate(nil, nil))
			for index, id := range []core.NodeID{"a", "b"} {
				c.create(ctx, contractRecord(id, new(core.NodeID("root")), uint64(index+1)), contractUpdate(nil, nil))
				c.close(ctx, id, contractCompleted(string(id)+" done"))
			}
			save := contractUpdate(nil, []core.Block{contractReceipt("a", 1)})
			rounds, err := save.CarriedCompletions()
			if err != nil || !reflect.DeepEqual(rounds, []core.CompletionID{{Child: "a", Round: 1}}) {
				c.t.Fatalf("receipts: %v %v", rounds, err)
			}
			c.save(ctx, "root", save)
			if !c.node(ctx, "a").Delivered || c.node(ctx, "b").Delivered {
				c.t.Fatal("save delivered wrong child rounds")
			}
			if err := c.store.MarkDelivered(ctx, "b", 1); err != nil {
				c.t.Fatal(err)
			}
			if !c.node(ctx, "b").Delivered {
				c.t.Fatal("explicit delivery not recorded")
			}
		}},
		{"reopen_makes_a_closed_node_live_with_its_message_and_delete_takes_the_subtree", func(ctx context.Context, c contractTree) {
			c.create(ctx, contractRecord("root", nil, 0), contractUpdate(nil, nil))
			c.create(ctx, contractRecord("child", new(core.NodeID("root")), 1), contractUpdate(nil, nil))
			c.create(ctx, contractRecord("grandchild", new(core.NodeID("child")), 2), contractUpdate(nil, nil))
			failed := store.NodeClose{Phase: &store.Failed{Failure: "boom"}, At: contractEpoch}
			c.close(ctx, "child", failed)
			if !reflect.DeepEqual(c.node(ctx, "child").Closed, &failed) {
				c.t.Fatal("close lost failure")
			}
			resumed := core.Timestamp("1970-01-01T00:01:00.000Z")
			if err := c.store.ReopenNode(ctx, "child", 2, resumed, contractMessage("m2")); err != nil {
				c.t.Fatal(err)
			}
			child := c.node(ctx, "child")
			if child.Closed != nil || child.Round != 2 || child.StartedAt != resumed || child.Delivered {
				c.t.Fatalf("reopened record: %#v", child)
			}
			if !reflect.DeepEqual(c.load(ctx, "child").State.Queue, []core.QueuedMessage{contractMessage("m2")}) {
				c.t.Fatal("reviving message not queued")
			}
			if err := c.store.DeleteNode(ctx, "child"); err != nil {
				c.t.Fatal(err)
			}
			for _, id := range []core.NodeID{"child", "grandchild"} {
				node, err := c.store.Node(ctx, id)
				if err != nil || node != nil {
					c.t.Fatalf("deleted node %s: %v %v", id, node, err)
				}
			}
			c.node(ctx, "root")
		}},
		{"a_completion_of_an_earlier_round_marks_the_current_one_undelivered", func(ctx context.Context, c contractTree) {
			c.create(ctx, contractRecord("root", nil, 0), contractUpdate(nil, nil))
			c.create(ctx, contractRecord("child", new(core.NodeID("root")), 1), contractUpdate(nil, nil))
			c.close(ctx, "child", contractCompleted("first"))
			if err := c.store.ReopenNode(ctx, "child", 2, contractEpoch, contractMessage("again")); err != nil {
				c.t.Fatal(err)
			}
			c.close(ctx, "child", contractCompleted("second"))
			c.save(ctx, "root", contractUpdate(nil, []core.Block{contractReceipt("child", 1)}))
			if err := c.store.MarkDelivered(ctx, "child", 1); err != nil {
				c.t.Fatal(err)
			}
			if c.node(ctx, "child").Delivered {
				c.t.Fatal("old completion delivered new round")
			}
			c.save(ctx, "root", contractUpdate(nil, []core.Block{contractReceipt("child", 1), contractReceipt("child", 2)}))
			if !c.node(ctx, "child").Delivered {
				c.t.Fatal("current completion not delivered")
			}
		}},
		{"each_sequence_gives_its_numbers_once_in_order", func(ctx context.Context, c contractTree) {
			sequences := []core.Sequence{"command", "command", "shell", "agent", "command", "shell"}
			want := []uint64{1, 2, 1, 1, 3, 2}
			for index, sequence := range sequences {
				number, err := c.store.NextNumber(ctx, sequence)
				if err != nil || number != want[index] {
					c.t.Fatalf("%s number: %d %v", sequence, number, err)
				}
			}
		}},
		{"the_blob_namespace_names_bytes_by_their_sha256", func(ctx context.Context, c contractTree) {
			blobs := c.store.SessionStore("root").Blobs()
			data := core.B64Bytes{0x89, 'P', 'N', 'G', 0, 1, 2, 3}
			blob, err := blobs.Put(ctx, data)
			if err != nil || string(blob) != fmt.Sprintf("%x", sha256.Sum256(data)) {
				c.t.Fatalf("blob name: %s %v", blob, err)
			}
			got, found, err := blobs.Read(ctx, blob)
			if err != nil || !found || !reflect.DeepEqual(got, data) {
				c.t.Fatalf("blob bytes: %v %v %v", got, found, err)
			}
			_, found, err = blobs.Read(ctx, core.BlobRef(fmt.Sprintf("%064d", 0)))
			if err != nil || found {
				c.t.Fatalf("unknown blob: %v %v", found, err)
			}
		}},
		{"a_save_delivers_a_completion_it_holds_as_waiting_input", func(ctx context.Context, c contractTree) {
			c.create(ctx, contractRecord("root", nil, 0), contractUpdate(nil, nil))
			c.create(ctx, contractRecord("child", new(core.NodeID("root")), 1), contractUpdate(nil, nil))
			c.close(ctx, "child", contractCompleted("child done"))
			update := contractUpdate(nil, nil)
			update.State.AgentInputs = append(update.State.AgentInputs, store.PendingAgentInput{TurnID: "waiting", Model: TestModel(), Message: contractReceipt("child", 1).Message})
			c.save(ctx, "root", update)
			if !c.node(ctx, "child").Delivered {
				c.t.Fatal("waiting completion not delivered")
			}
		}},
		{"children_list_in_spawn_order_and_a_close_keeps_its_result", func(ctx context.Context, c contractTree) {
			c.create(ctx, contractRecord("root", nil, 0), contractUpdate(nil, nil))
			for _, child := range []struct {
				id     core.NodeID
				number uint64
			}{{"late", 4}, {"early", 1}, {"second", 3}, {"first", 2}} {
				c.create(ctx, contractRecord(child.id, new(core.NodeID("root")), child.number), contractUpdate(nil, nil))
			}
			closed := store.NodeClose{Phase: &store.Completed{Result: "found it"}, At: "1970-01-01T00:01:30.000Z"}
			c.close(ctx, "early", closed)
			children, err := c.store.Children(ctx, "root")
			if err != nil {
				c.t.Fatal(err)
			}
			ids := []core.NodeID{}
			for _, child := range children {
				ids = append(ids, child.ID)
			}
			if !reflect.DeepEqual(ids, []core.NodeID{"early", "first", "second", "late"}) {
				c.t.Fatalf("spawn order: %v", ids)
			}
			early := c.node(ctx, "early")
			if !reflect.DeepEqual(early.Closed, &closed) || early.Delivered {
				c.t.Fatalf("closed record: %#v", early)
			}
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) { tc.run(t.Context(), contractTree{t: t, store: newStore(t)}) })
	}
}

const contractEpoch core.Timestamp = "1970-01-01T00:00:00.000Z"

// contractRecord creates a tree-contract node in its first round.
func contractRecord(id core.NodeID, parent *core.NodeID, number uint64) store.NodeRecord {
	record := store.RootRecord(id, contractEpoch)
	record.Parent = parent
	record.Number = number
	return record
}

// contractUpdate describes all rows of a checkpoint for a store-contract scenario.
func contractUpdate(queue []core.QueuedMessage, blocks []core.Block) store.CheckpointUpdate {
	if queue == nil {
		queue = []core.QueuedMessage{}
	}
	update := store.CheckpointUpdate{State: store.CheckpointState{Phase: "idle", Queue: queue, AgentInputs: []store.PendingAgentInput{}, Wakeups: []store.ScheduledWakeup{}, CWD: "/w", Model: TestModel(), Edits: []store.EditReceipt{}}, CommandState: new(store.InitialCommandState()), BlockCount: len(blocks), ChangedBlocks: []store.ChangedBlock{}}
	for index, block := range blocks {
		update.ChangedBlocks = append(update.ChangedBlocks, store.ChangedBlock{Index: index, Block: block})
	}
	return update
}

// contractMessage is a queued brief with the supplied turn identity.
func contractMessage(turn core.TurnID) core.QueuedMessage {
	return core.QueuedMessage{ID: turn, Content: Text("brief")}
}

// contractReceipt records one child's completion in the parent's transcript.
func contractReceipt(child core.NodeID, round uint64) *core.AgentMessageBlock {
	id := core.BlockID((core.CompletionID{Child: child, Round: round}).String())
	return &core.AgentMessageBlock{BlockID: id, TurnID: "turn", Timestamp: contractEpoch, Selection: TestModel(), Message: core.AgentMessage{ID: id, Sender: core.Sender{ID: child, Number: 1, Description: string(child), Round: round}, RecipientID: "root", Timestamp: contractEpoch, Content: string(child) + " done", Event: &core.CompletionEvent{Outcome: "completed"}}}
}

// contractCompleted closes a child with its final text.
func contractCompleted(result string) store.NodeClose {
	return store.NodeClose{Phase: &store.Completed{Result: result}, At: contractEpoch}
}

type contractTree struct {
	t     *testing.T
	store store.TreeStore
}

func (c contractTree) create(ctx context.Context, node store.NodeRecord, update store.CheckpointUpdate) {
	c.t.Helper()
	if err := c.store.CreateNode(ctx, node, update); err != nil {
		c.t.Fatal(err)
	}
}
func (c contractTree) save(ctx context.Context, id core.NodeID, update store.CheckpointUpdate) {
	c.t.Helper()
	if err := c.store.SessionStore(id).Save(ctx, update, store.CommitGuard{}); err != nil {
		c.t.Fatal(err)
	}
}
func (c contractTree) close(ctx context.Context, id core.NodeID, closed store.NodeClose) {
	c.t.Helper()
	if err := c.store.CloseNode(ctx, id, closed); err != nil {
		c.t.Fatal(err)
	}
}
func (c contractTree) node(ctx context.Context, id core.NodeID) *store.NodeRecord {
	c.t.Helper()
	node, err := c.store.Node(ctx, id)
	if err != nil || node == nil {
		c.t.Fatalf("node %s: %v %v", id, node, err)
	}
	return node
}
func (c contractTree) load(ctx context.Context, id core.NodeID) *store.Checkpoint {
	c.t.Helper()
	checkpoint, err := c.store.SessionStore(id).Load(ctx)
	if err != nil || checkpoint == nil {
		c.t.Fatalf("checkpoint %s: %v %v", id, checkpoint, err)
	}
	return checkpoint
}
