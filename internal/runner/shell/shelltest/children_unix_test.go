//go:build darwin || linux

package shelltest_test

import (
	"bytes"
	"errors"
	"runtime"
	"strconv"
	"strings"
	"testing"

	"golang.org/x/sys/unix"
)

func TestCancellationReapsExternalProgramsStartedByNativeUtilities(t *testing.T) {
	// Child readiness travels through stdout instead of polling a PID file.
	scripts := []string{
		`/bin/sh -c 'echo $$; exec /bin/sleep 60'`,
		`printf x | xargs /bin/sh -c 'echo $$; exec /bin/sleep 60'`,
		`find . -prune -exec /bin/sh -c 'echo $$; exec /bin/sleep 60' ';'`,
		`find . -prune -exec /bin/sh -c 'echo $$; exec /bin/sleep 60' sh '{}' +`,
	}
	scripts = append(scripts, `printf line | sed -n 'e /bin/sh -c "echo $$; exec /bin/sleep 60"'`)
	for _, script := range scripts {
		t.Run(script, func(t *testing.T) {
			if runtime.GOOS == "darwin" && strings.Contains(script, "sed -n") {
				t.Skip("decision 4: BSD sed does not support the e command")
			}
			if runtime.GOOS == "darwin" && (strings.Contains(script, "xargs") || strings.HasPrefix(script, "find")) {
				t.Skip("macOS has no child subreaper; Linux runs this scenario with one")
			}
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
			scope.Finish(ctx)
			if exit.Signal == nil || *exit.Signal != "SIGKILL" {
				t.Fatalf("exit %+v", exit)
			}
			if err := unix.Kill(pid, 0); !errors.Is(err, unix.ESRCH) {
				t.Fatalf("child %d remains after completion: %v", pid, err)
			}

		})
	}
}
