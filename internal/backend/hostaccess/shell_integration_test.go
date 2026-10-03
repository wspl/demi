package hostaccess

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerwire"
)

// Cost: in-process runner and local storage; completion is a runner event.
func TestConnectedNodeJobKeepsAdmissionThroughEditRetention(t *testing.T) {
	s := newTestShard(t)
	device := s.paired(t, "laptop")
	record := s.target(t, s.conversation(t), device, "/work")
	r := connectHost(t, s, device)
	identity, err := ConversationHostForNode(t.Context(), s, record.ID)
	if err != nil {
		t.Fatal(err)
	}
	pages := hosttest.NewPages(false)
	factory := NewShardShellEnvironments(s, s.native.Catalog(nil))
	environment, err := factory.Create(
		t.Context(),
		tools.EnvironmentScope{
			Root:     RootOf(record.ID),
			Node:     RootOf(record.ID),
			Agent:    1,
			Commands: &host.CommandSet{},
			Feed:     pages,
			Numbers:  &hosttest.CountingNumbers{},
		},
		identity,
	)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		// A failed assertion may leave a scripted job unanswered. Disconnect first
		// so disposing its environment can join every job without a fake kill reply.
		if err := r.connection.Close(context.Background()); err != nil {
			t.Error(err)
		}
		if err := environment.DisposeAll(context.Background()); err != nil {
			t.Error(err)
		}
	})
	result := startHostOperation(t, func(ctx context.Context) (host.CommandStatus, error) {
		return environment.Exec(
			ctx,
			host.ExecRequest{Script: "edit file", Caller: host.JobCaller{Node: RootOf(record.ID)}, ToolUseID: "call"},
		)
	})
	_ = nextHostMessage[*runnerwire.ManifestMessage](t, r)
	job := nextHostMessage[*runnerwire.JobStart](t, r)
	if job.CWD != "/work" {
		t.Fatal(job.CWD)
	}
	r.send(
		t,
		&runnerwire.JobExit{
			JobID:    job.JobID,
			ExitCode: new(int32(7)),
			Files: []runnerwire.JobFileChange{
				{
					Path:    "/work/file",
					Kind:    commandwire.EditModified,
					Added:   1,
					Removed: 1,
					Edits: []commandwire.EditCopies{
						{Original: new("/copies/before"), Modified: new("/copies/after")},
					},
				},
			},
		},
	)
	if path := sendHostRead(t, s, r, device, "before\n"); path != "/copies/before" {
		t.Fatal(path)
	}
	if held := s.conversations.Slot(record.ID).FileGate().TryReserve(); held != nil {
		held.Release()
		t.Fatal("job released admission before retaining edits")
	}
	if path := sendHostRead(t, s, r, device, "after\n"); path != "/copies/after" {
		t.Fatal(path)
	}
	release := nextHostMessage[*runnerwire.JobRelease](t, r)
	if release.JobID != job.JobID {
		t.Fatal(release)
	}
	completed := <-result
	if completed.err != nil {
		t.Fatal(completed.err)
	}
	status := completed.value
	if status.State.Phase != host.Exited || status.State.ExitCode != 7 || status.Files == nil ||
		len(status.Files.Files) != 1 {
		t.Fatal(status)
	}
	copies := status.Files.Files[0].Edits[0].Copies
	if copies == nil {
		t.Fatal("edit copies not retained")
	}
	for blob, expected := range map[core.BlobRef]string{copies.Original: "before\n", copies.Modified: "after\n"} {
		data, exists, err := s.blobs.Read(t.Context(), blob)
		if err != nil || !exists || string(data) != expected {
			t.Fatal(string(data), exists, err)
		}
	}
	if err := environment.DisposeAll(t.Context()); err != nil {
		t.Fatal(err)
	}
	held := s.conversations.Slot(record.ID).FileGate().TryReserve()
	if held == nil {
		t.Fatal("job retained admission after settlement")
	}
	held.Release()
	if s.jobs != 1 {
		t.Fatal("job completion omitted", s.jobs)
	}
}
