package server_test

import (
	"context"
	"errors"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/server/servertest"
	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func writeStorage(t *testing.T, f *fixture, value string, expected *host.Revision) host.StorageReply {
	t.Helper()
	reply, err := f.server.CommandStorage(
		t.Context(),
		rootID(),
		f.server.Node(rootID(), rootID()).JobCaller(),
		&host.StorageWriteIf{Key: "todo", Value: []byte(value), Expected: expected},
	)
	if err != nil {
		t.Fatal(err)
	}
	return reply
}

func TestForkKeepsCompletedPrefixFromLiveAndStoredRoot(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, said("answer A"), said("answer B"), said("answer U3"))
		c := f.opened()
		writeStorage(t, f, "1", new(host.Revision(0)))
		c.Send(t.Context(), send("m1", "A"))
		untilIdle(t, c)
		writeStorage(t, f, "2", new(host.Revision(1)))
		c.Send(t.Context(), send("m2", "B"))
		untilIdle(t, c)
		blocks := f.server.Tree(rootID()).Root().Session().Transcript().Blocks
		live, err := f.server.PrepareFork(t.Context(), rootID(), blocks[1].ID())
		if err != nil {
			t.Fatal(err)
		}
		if _, err := f.server.PrepareFork(
			t.Context(),
			rootID(),
			blocks[3].ID(),
		); !errors.Is(
			err,
			transcript.ErrNotCompletedText,
		) {
			t.Fatal("forked user block")
		}
		c.Send(t.Context(), &framewire.CloseFrame{})
		cold, err := f.server.PrepareFork(t.Context(), rootID(), blocks[1].ID())
		if err != nil {
			t.Fatal(err)
		}
		equal(t, live, cold)
		equal(t, blocks[:2], live.Transcript)
		equal(t, core.SessionPhaseIdle, live.State.Phase)
		equal(t, 0, len(live.State.Queue))
		equal(t, 0, len(live.State.Edits))
		destination, err := core.ParseNodeID("fork")
		if err != nil {
			t.Fatal(err)
		}
		running := cold
		running.State.Phase = core.SessionPhaseRunning
		err = f.server.InitializeFork(t.Context(), destination, running)
		var forkError *session.ForkError
		if !errors.As(err, &forkError) || forkError.Kind != session.ForkInvalidSeed {
			t.Fatal(err)
		}
		if err := f.server.InitializeFork(t.Context(), destination, live); err != nil {
			t.Fatal(err)
		}
		record := f.store.Record(destination)
		if record.Parent != nil || record.Closed != nil {
			t.Fatal(record)
		}
		forked := servertest.Connect(t, f.server, destination, "/workspace")
		forked.Send(t.Context(), &framewire.OpenFrame{})
		frames := forked.Received()
		equal(t, blocks[:2], frames[1].(*framewire.TranscriptResetFrame).Blocks)
		reply, err := f.server.CommandStorage(
			t.Context(),
			destination,
			f.server.Node(destination, destination).JobCaller(),
			&host.StorageRead{Key: "todo"},
		)
		if err != nil {
			t.Fatal(err)
		}
		equal(t, host.StorageReply(&host.StorageValue{Value: []byte("1"), Revision: 1}), reply)
		forked.Send(t.Context(), send("m3", "U3"))
		untilIdle(t, forked)
		equal(t, 0, f.script.Remaining())
		equal(
			t,
			[]provider.InferenceItem{
				&provider.UserMessage{Content: storetest.SentText("A")},
				&provider.AssistantText{ModelID: "test-model", Text: "answer A"},
				&provider.UserMessage{Content: storetest.SentText("U3")},
			},
			f.script.Requests()[2].Items,
		)
	})
}

func TestStorageRevisionAndRewriteEndOlderJobs(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, said("answer A"), said("answer A again"))
		c := f.opened()
		caller := f.server.Node(rootID(), rootID()).JobCaller()
		equal(t, host.StorageReply(&host.StorageCommitted{Revision: 1}), writeStorage(t, f, "1", new(host.Revision(0))))
		equal(t, host.StorageReply(&host.StorageConflict{Revision: 1}), writeStorage(t, f, "2", new(host.Revision(0))))
		equal(t, host.StorageReply(&host.StorageCommitted{Revision: 1}), writeStorage(t, f, "1", nil))
		c.Send(t.Context(), send("m1", "A"))
		untilIdle(t, c)
		versions := []uint64{}
		for _, version := range f.store.Checkpoint(rootID()).CommandState.Versions {
			versions = append(versions, version.Revision)
		}
		equal(t, []uint64{0, 1}, versions)
		c.Send(t.Context(), &framewire.RetryFrame{})
		untilIdle(t, c)
		_, err := f.server.CommandStorage(
			t.Context(),
			rootID(),
			caller,
			&host.StorageWriteIf{Key: "todo", Value: []byte("3")},
		)
		assertStorageError(t, err, "the command storage handle is no longer current")
		equal(t, host.StorageReply(&host.StorageCommitted{Revision: 2}), writeStorage(t, f, "3", nil))
		equal(t, "● 0  (root session) ← you\n", agent(t, f, rootID(), "list", `{}`).stdout)
		f.store.FailSaves(1)
		fresh := f.server.Node(rootID(), rootID()).JobCaller()
		_, err = f.server.CommandStorage(
			t.Context(),
			rootID(),
			fresh,
			&host.StorageWriteIf{Key: "todo", Value: []byte("4")},
		)
		assertStorageError(t, err, "the database refused the save")
		reply, err := f.server.CommandStorage(t.Context(), rootID(), fresh, &host.StorageRead{Key: "todo"})
		if err != nil {
			t.Fatal(err)
		}
		equal(t, host.StorageReply(&host.StorageValue{Value: []byte("3"), Revision: 2}), reply)
	})
}

func TestWriteBeforeAnswerBelongsToItsForkBoundary(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		finish := make(chan struct{})
		ended := make(chan struct{})
		stream := func(ctx context.Context, r provider.InferenceRequest) provider.Run {
			return func(yield func(provider.Event) bool) {
				if !yield(providertest.Text("streaming")) {
					return
				}
				select {
				case <-ctx.Done():
					return
				case <-finish:
				}
				close(ended)
				providertest.Events(providertest.Response(1, 1))(ctx, r)(yield)
			}
		}
		f := newFixture(t, stream)
		c := f.opened()
		equal(
			t,
			host.StorageReply(&host.StorageCommitted{Revision: 1}),
			writeStorage(t, f, `["T1"]`, new(host.Revision(0))),
		)
		c.Send(t.Context(), send("m1", "plan it"))
		synctest.Wait()
		gate := f.store.HoldSaves()
		defer gate.Release()
		result := make(chan struct {
			reply host.StorageReply
			err   error
		}, 1)
		caller := f.server.Node(rootID(), rootID()).JobCaller()
		go func() {
			reply, err := f.server.CommandStorage(
				t.Context(),
				rootID(),
				caller,
				&host.StorageWriteIf{Key: "todo", Value: []byte(`["T1","T2"]`), Expected: new(host.Revision(1))},
			)
			result <- struct {
				reply host.StorageReply
				err   error
			}{reply, err}
		}()
		if err := gate.Wait(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		close(finish)
		<-ended
		synctest.Wait()
		answer := f.server.Tree(rootID()).Root().Session().Transcript().Blocks[1].(*core.TextBlock)
		if answer.Forkable {
			t.Fatal("answer completed before ordered storage write")
		}
		gate.Release()
		committed := <-result
		if committed.err != nil {
			t.Fatal(committed.err)
		}
		equal(t, host.StorageReply(&host.StorageCommitted{Revision: 2}), committed.reply)
		untilIdle(t, c)
		if !f.server.Tree(rootID()).Root().Session().Transcript().Blocks[1].(*core.TextBlock).Forkable {
			t.Fatal("answer not completed")
		}
		equal(
			t,
			host.StorageReply(&host.StorageCommitted{Revision: 3}),
			writeStorage(t, f, `["T1","T2","T3"]`, new(host.Revision(2))),
		)
		seed, err := f.server.PrepareFork(t.Context(), rootID(), answer.ID())
		if err != nil {
			t.Fatal(err)
		}
		equal(t, uint64(2), seed.CommandState.Revision)
	})
}
