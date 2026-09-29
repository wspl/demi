//go:build linux

// Package roottest runs the machine manager's tests that need root. They run only
// when asked for (DEMI_TEST_ROOT=1, as root) and never touch the execution host's
// state: the test binary starts itself again in new mount and network
// namespaces, with every mount private and a fresh tmpfs over /run, so the
// namespace files and runtime directories of the host's manager stay out of
// reach and the test's mounts, loop devices' mounts, interfaces and firewall
// tables end with it.
package roottest

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"

	"golang.org/x/sys/unix"
)

const (
	enabled  = "DEMI_TEST_ROOT"
	isolated = "DEMI_TEST_ISOLATED"
)

// Main is a package's TestMain: it isolates the test binary when root tests are
// enabled, and runs the tests.
func Main(m *testing.M) int {
	if os.Getenv(enabled) != "1" {
		return m.Run()
	}
	if os.Getenv(isolated) == "1" {
		if err := isolate(); err != nil {
			os.Stderr.WriteString("roottest: " + err.Error() + "\n")
			return 1
		}
		return m.Run()
	}
	if os.Geteuid() != 0 {
		os.Stderr.WriteString("roottest: " + enabled + "=1 needs root\n")
		return 1
	}
	command := exec.Command(os.Args[0], os.Args[1:]...)
	command.Env = append(os.Environ(), isolated+"=1")
	command.Stdin, command.Stdout, command.Stderr = os.Stdin, os.Stdout, os.Stderr
	command.SysProcAttr = &syscall.SysProcAttr{Cloneflags: syscall.CLONE_NEWNS | syscall.CLONE_NEWNET}
	if err := command.Run(); err != nil {
		if exit, ok := err.(*exec.ExitError); ok {
			return exit.ExitCode()
		}
		os.Stderr.WriteString("roottest: " + err.Error() + "\n")
		return 1
	}
	return 0
}

// isolate makes every mount private and gives the process its own /run.
func isolate() error {
	if err := unix.Mount("", "/", "", unix.MS_REC|unix.MS_PRIVATE, ""); err != nil {
		return err
	}
	return unix.Mount("tmpfs", "/run", "tmpfs", 0, "")
}

// Require skips the test unless root tests are enabled.
func Require(t testing.TB) {
	t.Helper()
	if os.Getenv(enabled) != "1" {
		t.Skip("needs root: run with " + enabled + "=1 as root")
	}
}

// LoopAttached reports whether a loop device has image as its backing file.
func LoopAttached(t testing.TB, image string) bool {
	t.Helper()
	image, err := filepath.EvalSymlinks(image)
	if err != nil {
		t.Fatal(err)
	}
	backings, err := filepath.Glob("/sys/block/loop*/loop/backing_file")
	if err != nil {
		t.Fatal(err)
	}
	for _, backing := range backings {
		if data, err := os.ReadFile(backing); err == nil && strings.TrimSpace(string(data)) == image {
			return true
		}
	}
	return false
}

// LoopDetaches reports whether the loop device of image detaches within five
// seconds: the kernel runs an auto-clear detach after the last user closes it, and
// announces it nowhere a test can wait for.
func LoopDetaches(t testing.TB, image string) bool {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for LoopAttached(t, image) {
		if time.Now().After(deadline) {
			return false
		}
		time.Sleep(20 * time.Millisecond)
	}
	return true
}
