//go:build unix

package jobs_test

import (
	"testing"

	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runnerwire"
)

// One raw process; completion is event-driven and cancellation must reap it.
func TestRawProcessKilledImmediatelyReportsSIGKILL(t *testing.T) {
	table, output, _, root := newTable(t, 8, nil)
	if err := table.Start(jobs.TaskSpec{ID: "raw", Cwd: root, Command: &jobs.ProcessCommand{Command: "/bin/sleep", Args: []string{"10"}}}); err != nil {
		t.Fatal(err)
	}
	if err := table.Signal(jobs.WorkID{Kind: jobs.ProcessWork, ID: "raw"}, runnerwire.SignalKill); err != nil {
		t.Fatal(err)
	}
	exit, ok := reply(t, output).(*runnerwire.SpawnExit)
	if !ok || exit.SpawnError != nil || exit.ExitCode != nil || exit.Signal == nil || *exit.Signal != "SIGKILL" {
		t.Fatalf("immediate kill: %+v", exit)
	}
}
