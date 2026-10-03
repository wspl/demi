//go:build darwin || linux

package engine

import (
	"context"
	"errors"
	"io"
	"os"
	"os/exec"
	"runtime"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdsdk/cmdsdktest"
	"golang.org/x/sys/unix"
)

// Descriptor exhaustion changes process-wide limits, so it runs in a child test
// executable. Each case waits for an observed retry, normally below 100 ms.
func TestPipelineWaitsForDescriptors(t *testing.T) {
	if os.Getenv("DEMI_SHELL_EXHAUSTION") != "1" {
		executable, err := os.Executable()
		if err != nil {
			t.Fatal(err)
		}
		command := exec.CommandContext(
			t.Context(),
			executable,
			"-test.run=^TestPipelineWaitsForDescriptors$",
			"-test.timeout=15s",
		)
		command.Env = append(os.Environ(), "DEMI_SHELL_EXHAUSTION=1")
		if out, err := command.CombinedOutput(); err != nil {
			t.Fatalf("%v\n%s", err, out)
		}
		return
	}
	var limit unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &limit); err != nil {
		t.Fatal(err)
	}
	limit.Cur = 128
	if err := unix.Setrlimit(unix.RLIMIT_NOFILE, &limit); err != nil {
		t.Fatal(err)
	}
	for _, cancelled := range []bool{false, true} {
		root := t.TempDir()
		var held []*os.File
		for {
			f, err := os.Open(os.DevNull)
			if errors.Is(err, unix.EMFILE) {
				break
			}
			if err != nil {
				t.Fatal(err)
			}
			held = append(held, f)
		}
		release := func() {
			for _, f := range held {
				_ = f.Close()
			}
			held = nil
		}
		ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
		done := make(chan struct{})
		var runErr error
		baseline := cmdsdktest.Pauses()
		go func() {
			result, err := Execute(ctx, "true | true", Options{Cwd: root, Stdout: io.Discard, Stderr: io.Discard})
			if err == nil && result.Code != 0 {
				err = errors.New("pipeline failed")
			}
			runErr = err
			close(done)
		}()
		defer func() {
			cancel()
			release()
			<-done
		}()
		for cmdsdktest.Pauses() == baseline {
			select {
			case <-done:
				release()
				cancel()
				t.Fatalf("pipeline did not wait: %v", runErr)
			case <-ctx.Done():
				release()
				cancel()
				t.Fatal(ctx.Err())
			default:
				runtime.Gosched()
			}
		}
		if cancelled {
			cancel()
		} else {
			release()
		}
		<-done
		err := runErr
		release()
		cancel()
		if cancelled && !errors.Is(err, context.Canceled) {
			t.Fatalf("cancelled pipeline: %v", err)
		}
		if !cancelled && err != nil {
			t.Fatal(err)
		}
	}
}
