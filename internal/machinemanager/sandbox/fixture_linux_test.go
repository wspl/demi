//go:build linux

package sandbox

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"testing"

	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	if len(os.Args) > 1 && os.Args[1] == "--" {
		if err := guestProbe(); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		os.Exit(0)
	}
	if len(os.Args) > 1 && (strings.HasPrefix(os.Args[1], "--root=") || os.Args[1] == "--version") {
		if err := runscFixture(os.Args[1:]); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		os.Exit(0)
	}
	if len(os.Args) > 1 && os.Args[len(os.Args)-1] == "--recover-namespace" {
		if err := recoveryProbe(os.Args[1:]); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		os.Exit(0)
	}
	goleak.VerifyTestMain(m)
}

// runscFixture models the runtime protocol, not gVisor: starts can fail, wait
// blocks on a socket event, and commands update a container's observable state.
func runscFixture(args []string) error {
	if args[0] == "--version" {
		_, err := fmt.Fprintln(os.Stdout, "runsc version "+PinnedRelease().Version())
		return err
	}
	root := strings.TrimPrefix(args[0], "--root=")
	for len(args) > 0 && strings.HasPrefix(args[0], "--") {
		args = args[1:]
	}
	if len(args) == 0 {
		return errors.New("fixture needs a runtime command")
	}
	trace, err := os.OpenFile(filepath.Join(root, "trace"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	_, writeErr := fmt.Fprintln(trace, args[0])
	if err := errors.Join(writeErr, trace.Close()); err != nil {
		return err
	}
	state := filepath.Join(root, "state")
	switch args[0] {
	case "list":
		data, err := os.ReadFile(state)
		if errors.Is(err, os.ErrNotExist) {
			_, err := fmt.Fprint(os.Stdout, "null")
			return err
		}
		if err != nil {
			return err
		}
		fields := strings.Fields(string(data))
		if len(fields) != 2 {
			return errors.New("invalid fixture state")
		}
		listing, err := (runtimeListing{{ID: fields[0], Status: Status(fields[1])}}).MarshalJSON()
		if err != nil {
			return err
		}
		_, err = os.Stdout.Write(listing)
		return err
	case "run":
		if _, err := os.Stat(filepath.Join(root, "fail-start")); err == nil {
			return errors.New("fixture failed start")
		}
		return os.WriteFile(state, []byte(args[len(args)-1]+" running"), 0o600)
	case "wait":
		connection, err := net.Dial("unix", filepath.Join(root, "control"))
		if err != nil {
			if errors.Is(err, os.ErrNotExist) {
				return nil
			}
			return err
		}
		defer func() {
			_ = connection.Close()
		}() // No buffered writes remain after the protocol event.
		if _, err := connection.Write([]byte("W")); err != nil {
			return err
		}
		var release [1]byte
		_, err = connection.Read(release[:])
		return err
	case "pause", "resume", "kill":
		data, err := os.ReadFile(state)
		if err != nil {
			return err
		}
		id, _, _ := strings.Cut(string(data), " ")
		status := "paused"
		if args[0] == "resume" {
			status = "running"
		}
		if args[0] == "kill" {
			status = "stopped"
		}
		return os.WriteFile(state, []byte(id+" "+status), 0o600)
	case "delete":
		err := os.Remove(state)
		if errors.Is(err, os.ErrNotExist) {
			return nil
		}
		return err
	default:
		return fmt.Errorf("unknown fixture command %q", args[0])
	}
}

// recoveryProbe verifies namespace inheritance on multiple child runtime threads
// and the borrowed state lock at fd 3. An explicit failure tests handle retention.
func recoveryProbe(args []string) error {
	if len(args) != 5 || args[0] != "--probe" {
		return fmt.Errorf("recovery arguments = %q", args)
	}
	lock := os.NewFile(RecoveryLockFD, "inherited lock")
	if lock == nil {
		return errors.New("missing inherited lock")
	}
	defer func() {
		_ = lock.Close()
	}() // Borrowed read-only lock has no buffered writes.
	actual, err := lock.Stat()
	if err != nil {
		return err
	}
	expected, err := os.Stat(args[2])
	if err != nil {
		return err
	}
	if !os.SameFile(actual, expected) {
		return errors.New("wrong inherited lock")
	}
	if err := syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		return err
	}
	var workers sync.WaitGroup
	failures := make(chan error, 8)
	for range 8 {
		workers.Go(func() {
			data, err := os.ReadFile(args[1])
			if err != nil {
				failures <- err
			} else if string(data) != "inside saved namespace" {
				failures <- fmt.Errorf("namespace content = %q", data)
			}
		})
	}
	workers.Wait()
	close(failures)
	for err := range failures {
		return err
	}
	fail, err := strconv.ParseBool(args[3])
	if err != nil {
		return err
	}
	if fail {
		return errors.New("requested recovery failure")
	}
	return nil
}

// fixtureCopy copies bytes only for the scripted disk dependency; production
// sparse copying belongs to storage and is never implemented in sandbox.
func fixtureCopy(_ context.Context, source, destination string) (err error) {
	input, err := os.Open(source)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, input.Close())
	}()
	output, err := os.Create(destination)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, output.Close())
	}()
	_, err = io.Copy(output, input)
	return err
}
