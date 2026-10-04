package shelltest_test

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runner/shell"
	"github.com/wspl/demi/internal/runner/shell/shelltest"
	"github.com/wspl/demi/internal/runnerwire"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	if handled, err := process.RunChildBootstrap(); handled {
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		os.Exit(0)
	}
	goleak.VerifyTestMain(m)
}

// shellJob creates a real login job with an isolated home and mandatory joined cleanup.
func shellJob(t *testing.T, script string) (context.Context, *shelltest.Scope, process.ShellJob, string) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	t.Cleanup(cancel)
	root := t.TempDir()
	scope := shelltest.NewScope(ctx, nil)
	t.Cleanup(func() {
		scope.Cancel()
		scope.Finish(context.Background())
	})
	job, err := scope.Start(
		ctx,
		script,
		root,
		map[string]string{"HOME": root, "PATH": os.Getenv("PATH"), "TMPDIR": root},
		true,
	)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		job.Cancel()
		_, _, _ = job.Wait(context.Background())
	})
	return ctx, scope, job, root
}

// marker observes exactly the expected prefix without waiting for a time window.
func marker(ctx context.Context, t *testing.T, job process.ShellJob, want string) {
	t.Helper()
	var output []byte
	for len(output) < len(want) {
		select {
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		case chunk, ok := <-job.Output():
			if !ok {
				exit, _, _ := job.Wait(ctx)
				t.Fatalf("job ended before %q: %+v", want, exit)
			}
			if chunk.Stream != runnerwire.Stdout {
				t.Fatalf("stderr before marker: %s", chunk.Bytes)
			}
			output = append(output, chunk.Bytes...)
		}
	}
	if string(output) != want {
		t.Fatalf("marker %q want %q", output, want)
	}
}

func TestShellCancellationReportsTheRequestingSignal(t *testing.T) {
	for _, signal := range []runnerwire.Signal{
		runnerwire.SignalTerminate,
		runnerwire.SignalInterrupt,
		runnerwire.SignalHangup,
		runnerwire.SignalQuit,
		runnerwire.SignalKill,
		"",
	} {
		t.Run(string(signal), func(t *testing.T) {
			t.Parallel()
			ctx, scope, job, _ := shellJob(t, "printf ready; sleep 60")
			marker(ctx, t, job, "ready")
			if err := job.Signal(runnerwire.SignalUser1); err == nil {
				t.Fatal("accepted unsupported signal")
			}
			if job.IsCancelled() {
				t.Fatal("unsupported signal cancelled job")
			}
			if signal == "" {
				job.Cancel()
				if err := job.Signal(runnerwire.SignalTerminate); err != nil {
					t.Fatal(err)
				}
			} else {
				if err := job.Signal(signal); err != nil {
					t.Fatal(err)
				}
				if err := job.Signal(runnerwire.SignalKill); err != nil {
					t.Fatal(err)
				}
			}
			exit, _, err := job.Wait(ctx)
			want := string(signal)
			if want == "" {
				want = "SIGKILL"
			}
			if exit.Signal == nil || *exit.Signal != want || exit.Code != nil || err != nil {
				t.Fatalf("exit %+v want %s", exit, want)
			}
			scope.Finish(ctx)
		})
	}
}

func TestJobsShareTheRunnerProcessAndCancellationIsIsolated(t *testing.T) {
	ctx, _, sibling, _ := shellJob(t, `printf '%s' $$; read go; printf done`)
	marker(ctx, t, sibling, strconv.Itoa(os.Getpid()))
	scripts := []string{
		"while :; do :; done",
		// Separate expressions keep the label portable to BSD sed; otherwise
		// it treats the branch as part of the label and exits instead of looping.
		`printf 'line\n' | sed -e ':again' -e 'b again'`,
		"cat", "tee file", "wc -c", "head -c 99999", "tail -c +1", "od -j 99999",
		"(sleep 60) & wait", "cat <(sleep 60)", "touch file; tail -f -s 60 file",
		"while :; do printf 'a long line to fill the output pipe\n'; done",
	}
	scripts = append(scripts, "jq -n 'def spin: spin; spin'")
	t.Run("blocked", func(t *testing.T) {
		for _, script := range scripts {
			t.Run(script, func(t *testing.T) {
				t.Parallel()
				if script == "jq -n 'def spin: spin; spin'" {
					if _, err := exec.LookPath("jq"); err != nil {
						t.Skip("decision 4: system jq unavailable; utility provisioning is deferred")
					}
				}
				// Gate the additional output-pressure case so its ready marker is a
				// separate observation even when the OS coalesces adjacent writes.
				prelude := "printf ready; "
				pressure := script == "while :; do printf 'a long line to fill the output pipe\n'; done"
				if pressure {
					prelude += "read go; "
				}
				ctx, scope, job, _ := shellJob(t, prelude+script)
				marker(ctx, t, job, "ready")
				if pressure {
					job.Input() <- process.Input{Bytes: []byte("go\n")}
				}
				if err := scope.Activity().WaitBlockedOrChecks(ctx, scope.Activity().Checks()+100); err != nil {
					t.Fatal(err)
				}
				job.Cancel()
				exit, _, _ := job.Wait(ctx)
				if exit.Signal == nil || *exit.Signal != "SIGKILL" {
					t.Fatalf("exit %+v", exit)
				}
				scope.Finish(ctx)
			})
		}
	})
	sibling.Input() <- process.Input{Bytes: []byte("go\n")}
	sibling.Input() <- process.Input{}
	var output []byte
	for chunk := range sibling.Output() {
		if chunk.Stream != runnerwire.Stdout {
			t.Fatalf("unexpected sibling stderr: %q", chunk.Bytes)
		}
		output = append(output, chunk.Bytes...)
	}
	exit, _, _ := sibling.Wait(ctx)
	if exit.Code == nil || *exit.Code != 0 || string(output) != "done" {
		t.Fatalf("sibling exit %+v output %q", exit, output)
	}
}

func TestJobCompletionPreservesProcessSubstitutionOutput(t *testing.T) {
	// Supply data through stdin, so no wall-time sleep orders the producer/consumer.
	ctx, scope, job, root := shellJob(t, `cat | tee >(cat > copied) > /dev/null`)
	content := bytes.Repeat([]byte{'x'}, 256*1024)
	select {
	case job.Input() <- process.Input{Bytes: content}:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	select {
	case job.Input() <- process.Input{}:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	for chunk := range job.Output() {
		if len(chunk.Bytes) > 0 {
			t.Fatalf("unexpected output %s", chunk.Bytes)
		}
	}
	exit, _, _ := job.Wait(ctx)
	scope.Finish(ctx)
	data, err := os.ReadFile(filepath.Join(root, "copied"))
	if err != nil || exit.Code == nil || *exit.Code != 0 || !bytes.Equal(data, content) {
		t.Fatalf("exit %+v bytes %d (%v)", exit, len(data), err)
	}
}

func TestPublicShellStartsFreshJobs(t *testing.T) {
	root := t.TempDir()
	job, err := (&shell.Shell{}).
		Start(t.Context(), process.JobStart{
			Script: "printf public",
			Cwd:    root,
			Env:    map[string]string{"HOME": root, "PATH": os.Getenv("PATH")},
		})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		job.Cancel()
		_, _, _ = job.Wait(context.Background())
	}()
	var output []byte
	for chunk := range job.Output() {
		output = append(output, chunk.Bytes...)
	}
	exit, cwd, _ := job.Wait(t.Context())
	if exit.Code == nil || *exit.Code != 0 || cwd == nil || *cwd != root || string(output) != "public" ||
		job.IsCancelled() {
		t.Fatalf("exit %+v cwd %v output %q", exit, cwd, output)
	}
}

func TestJobJoinsBackgroundTasksAndPreservesForegroundExitStatus(t *testing.T) {
	ctx, scope, job, root := shellJob(t, `(printf waiting; read release; echo completed > done) & exit 7`)
	marker(ctx, t, job, "waiting")
	select {
	case job.Input() <- process.Input{Bytes: []byte("release\n")}:
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	for range job.Output() {
	}
	exit, _, _ := job.Wait(ctx)
	scope.Finish(ctx)
	data, err := os.ReadFile(filepath.Join(root, "done"))
	if err != nil || exit.Code == nil || *exit.Code != 7 || string(data) != "completed\n" {
		t.Fatalf("exit %+v output %q (%v)", exit, data, err)
	}
}
