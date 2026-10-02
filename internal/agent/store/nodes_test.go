package store_test

import (
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

func TestNodeJobTracksItsRoundAndClose(t *testing.T) {
	root := store.RootRecord("root", "1970-01-01T00:00:00.000Z")
	if root.Job() != nil || root.Number != 0 || root.Round != 1 || !root.CanSpawnSubagents {
		t.Fatalf("root: %#v", root)
	}
	child := root
	child.ID = "child"
	child.Parent = new(core.NodeID("root"))
	child.Description = "find it"
	child.Profile = new("research")
	running := child.Job()
	if running.Phase != framewire.JobPhaseRunning || running.EndedAt != nil || running.Result != nil || running.SubagentID != "child" || running.ParentSessionID != "root" {
		t.Fatalf("running job: %#v", running)
	}
	*running.Profile = "changed"
	if *child.Profile != "research" {
		t.Fatal("job shares profile pointer")
	}
	for _, tc := range []struct {
		phase  store.ClosePhase
		want   framewire.JobPhase
		result *string
	}{{&store.Completed{Result: "found it"}, framewire.JobPhaseCompleted, new("found it")}, {&store.Aborted{}, framewire.JobPhaseAborted, nil}, {&store.Failed{Failure: "failed"}, framewire.JobPhaseError, nil}} {
		child.Closed = &store.NodeClose{Phase: tc.phase, At: "1970-01-01T00:01:00.000Z"}
		job := child.Job()
		if job.Phase != tc.want || job.EndedAt == nil || *job.EndedAt != child.Closed.At || (job.Result == nil) != (tc.result == nil) {
			t.Fatalf("closed job: %#v", job)
		}
		if tc.result != nil && *tc.result != *job.Result {
			t.Fatal("completion lost final text")
		}
	}
}
