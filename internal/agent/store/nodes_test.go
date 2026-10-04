package store_test

import (
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
)

func TestNodeJobTracksItsRoundAndClose(t *testing.T) {
	root := store.RootRecord("root", "1970-01-01T00:00:00.000Z")
	if _, ok := root.Job(); ok || root.Number != 0 || root.Round != 1 || !root.CanSpawnSubagents {
		t.Fatalf("root: %#v", root)
	}
	child := root
	child.ID = "child"
	child.Parent = new(types.NodeID("root"))
	child.Description = "find it"
	child.Profile = new("research")
	running, _ := child.Job()
	if running.Phase != conversationproto.JobPhaseRunning || running.EndedAt != nil || running.Result != nil ||
		running.SubagentID != "child" ||
		running.ParentSessionID != "root" {
		t.Fatalf("running job: %#v", running)
	}
	*running.Profile = "changed"
	if *child.Profile != "research" {
		t.Fatal("job shares profile pointer")
	}
	for _, scenario := range []struct {
		phase  store.ClosePhase
		want   conversationproto.JobPhase
		result *string
	}{
		{
			&store.Completed{Result: "found it"},
			conversationproto.JobPhaseCompleted,
			new("found it"),
		},
		{
			&store.Aborted{},
			conversationproto.JobPhaseAborted,
			nil,
		},
		{
			&store.Failed{Failure: "failed"},
			conversationproto.JobPhaseError,
			nil,
		},
	} {
		child.Closed = &store.NodeClose{Phase: scenario.phase, At: "1970-01-01T00:01:00.000Z"}
		job, _ := child.Job()
		if job.Phase != scenario.want || job.EndedAt == nil || *job.EndedAt != child.Closed.At ||
			(job.Result == nil) != (scenario.result == nil) {
			t.Fatalf("closed job: %#v", job)
		}
		if scenario.result != nil && *scenario.result != *job.Result {
			t.Fatal("completion lost final text")
		}
	}
}
