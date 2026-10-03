//go:build darwin || linux

package shelltest_test

import (
	"bytes"
	"errors"
	"strconv"
	"strings"
	"testing"

	"golang.org/x/sys/unix"
)

func TestCancellationReapsExternalProgramsStartedByNativeUtilities(t *testing.T) {
	// Real utility processes, normally below one second. Readiness travels
	// through stdout; concurrent jobs also exercise independent group ownership.
	scripts := []string{
		`/bin/sh -c 'echo $$; exec /bin/sleep 60'`,
		`printf x | xargs /bin/sh -c 'echo $$; exec /bin/sleep 60'`,
		`find . -prune -exec /bin/sh -c 'echo $$; exec /bin/sleep 60' ';'`,
		`find . -prune -exec /bin/sh -c 'echo $$; exec /bin/sleep 60' sh '{}' +`,
	}
	scripts = append(scripts, `printf line | sed -n 'e /bin/sh -c "echo $$; exec /bin/sleep 60"'`)
	for _, script := range scripts {
		t.Run(script, func(t *testing.T) {
			// Rust's embedded sed streamed the e command's output as it ran.
			// BSD sed has no e command, and GNU sed prints its output only
			// after the command ends, so the child's ready line never comes.
			if strings.Contains(script, "sed -n") {
				t.Skip("decision 4: system sed does not stream the e command's output (BSD lacks e; GNU waits for the command)")
			}
			t.Parallel()
			ctx, scope, job, _ := shellJob(t, script)
			var line []byte
			for !bytes.ContainsRune(line, '\n') {
				select {
				case chunk, ok := <-job.Output():
					if !ok {
						t.Fatal("child exited before readiness")
					}
					line = append(line, chunk.Bytes...)
				case <-ctx.Done():
					t.Fatal(ctx.Err())
				}
			}
			pid, err := strconv.Atoi(string(bytes.TrimSpace(line)))
			if err != nil {
				t.Fatal(err)
			}
			job.Cancel()
			exit, _ := job.Wait(ctx)
			if exit.Signal == nil || *exit.Signal != "SIGKILL" || exit.Error != nil {
				t.Fatalf("exit %+v", exit)
			}
			if err := unix.Kill(pid, 0); !errors.Is(err, unix.ESRCH) {
				t.Fatalf("child %d remains after completion: %v", pid, err)
			}
			scope.Finish(ctx)
		})
	}
}
