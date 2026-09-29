package process_test

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"testing"

	"github.com/wspl/demi/go/runner/process"
)

// One native child and one retry pause; the writable executable is released
// only after the kernel has reported ETXTBSY, never after an elapsed delay.
func TestStartExecutableAfterWriterCloses(t *testing.T) {
	data, err := os.ReadFile("/bin/true")
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "program")
	if err := os.WriteFile(path, data, 0700); err != nil {
		t.Fatal(err)
	}
	writer, err := os.OpenFile(path, os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	defer writer.Close()
	attempts := 0
	cmd, err := process.Start(t.Context(), func() (*exec.Cmd, error) {
		attempts++
		cmd := exec.CommandContext(t.Context(), path)
		err := cmd.Start()
		if attempts == 1 {
			if !errors.Is(err, syscall.ETXTBSY) {
				t.Fatalf("held executable: %v", err)
			}
			if err := writer.Close(); err != nil {
				t.Fatal(err)
			}
		}
		return cmd, err
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := cmd.Wait(); err != nil {
		t.Fatal(err)
	}
	if attempts != 2 {
		t.Fatalf("attempts=%d", attempts)
	}
}
