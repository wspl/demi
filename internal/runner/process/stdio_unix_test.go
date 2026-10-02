//go:build darwin || linux

package process

import (
	"os"
	"testing"

	"golang.org/x/sys/unix"
)

func TestStandardDuplicatesAndLiveInput(t *testing.T) {
	standard, err := StandardFile(t.Context(), 1)
	if err != nil {
		t.Fatal(err)
	}
	if standard.Fd() == os.Stdout.Fd() {
		t.Fatal("standard handle was not duplicated")
	}
	if err := standard.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stdout.Stat(); err != nil {
		t.Fatalf("closing duplicate closed stdout: %v", err)
	}
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = reader.Close(); _ = writer.Close() }()
	reference, err := LiveReference(reader)
	if err != nil {
		t.Fatal(err)
	}
	duplicate, err := unix.FcntlInt(reader.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		t.Fatal(err)
	}
	cloned := os.NewFile(uintptr(duplicate), "live clone")
	defer func() { _ = cloned.Close() }()
	live, err := IsLive(cloned, map[string]string{LiveInputEnv: reference})
	if err != nil || !live {
		t.Fatalf("cloned live input: %t %v", live, err)
	}
	live, err = IsLive(os.Stdout, map[string]string{LiveInputEnv: reference})
	if err != nil || live {
		t.Fatalf("unrelated input: %t %v", live, err)
	}
	live, err = IsLive(reader, nil)
	if err != nil || live {
		t.Fatalf("absent reference: %t %v", live, err)
	}
}
