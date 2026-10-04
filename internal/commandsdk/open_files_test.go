//go:build darwin || linux

package commandsdk

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"syscall"
	"testing"

	"golang.org/x/sys/unix"
)

// This test owns an isolated subprocess because descriptor limits are process-wide.
// Cost: one test-binary launch, no build, a 30-second failure budget from the parent suite.
func TestRecordingEditWaitsForOpenFile(t *testing.T) {
	if os.Getenv("DEMI_CMDSDK_EXHAUST_CHILD") != "1" {
		cmd := exec.CommandContext(
			t.Context(),
			os.Args[0],
			"-test.run=^TestRecordingEditWaitsForOpenFile$",
			"-test.timeout=30s",
		)
		cmd.Env = append(os.Environ(), "DEMI_CMDSDK_EXHAUST_CHILD=1")
		b, err := cmd.CombinedOutput()
		if err != nil {
			t.Fatalf("descriptor child: %v\n%s", err, b)
		}
		return
	}
	root := t.TempDir()
	r := recorder(t, root)
	path := filepath.Join(root, "file")
	must(t, os.WriteFile(path, []byte("before"), 0o600))
	var limit unix.Rlimit
	must(t, unix.Getrlimit(unix.RLIMIT_NOFILE, &limit))
	old := limit
	limit.Cur = 128
	must(t, unix.Setrlimit(unix.RLIMIT_NOFILE, &limit))
	defer func() {
		must(t, unix.Setrlimit(unix.RLIMIT_NOFILE, &old))
	}()
	var held []*os.File
	defer func() {
		for _, f := range held {
			must(t, f.Close())
		}
	}()
	for {
		f, err := os.Open(os.DevNull)
		if errors.Is(err, syscall.EMFILE) {
			break
		}
		must(t, err)
		held = append(held, f)
	}
	before := DescriptorPauses()
	done := make(chan error, 1)
	go func() {
		done <- r.Record(t.Context(), path, func() error {
			_, err := Retry(t.Context(), func() (struct{}, error) {
				return struct{}{}, os.WriteFile(path, []byte("after"), 0o600)
			})
			return err
		})
	}()
	for DescriptorPauses() == before {
		runtime.Gosched()
	}
	select {
	case err := <-done:
		t.Fatalf("recording ended without descriptor: %v", err)
	default:
	}
	keep := max(len(held)-64, 0)
	for _, f := range held[keep:] {
		must(t, f.Close())
	}
	held = held[:keep]
	must(t, <-done)
	j := report(t, r)
	if len(j.Files) != 1 {
		t.Fatalf("lost edit: %+v", j)
	}
	if snapshot(t, j.Files[0].Edits[0].Original) != "before" || snapshot(t, j.Files[0].Edits[0].Modified) != "after" {
		t.Fatal("wrong snapshots")
	}
}
