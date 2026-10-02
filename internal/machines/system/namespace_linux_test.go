//go:build linux

package system_test

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machines/system/systemtest"
)

func TestNamespaceJobsStayIsolated(t *testing.T) {
	if os.Geteuid() != 0 {
		t.Skip("needs explicit root invocation")
	}
	before, err := os.Stat("/proc/self/ns/net")
	if err != nil {
		t.Fatal(err)
	}
	_, err = system.RunNamespace(t.Context(), system.Network(""), func(context.Context) (struct{}, error) {
		t.Error("empty network path created a namespace instead of failing")
		return struct{}{}, nil
	})
	if !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("empty namespace path: %v", err)
	}
	failure := errors.New("job failure")
	for _, mode := range []string{"success", "error", "panic"} {
		_, err := system.RunNamespace(t.Context(), system.NewNetwork(), func(ctx context.Context) (struct{}, error) {
			current, err := os.Stat("/proc/thread-self/ns/net")
			if err != nil {
				return struct{}{}, err
			}
			if os.SameFile(before, current) {
				return struct{}{}, fmt.Errorf("job retained parent network namespace")
			}
			if mode == "panic" {
				panic(failure)
			}
			if mode == "error" {
				return struct{}{}, failure
			}
			return struct{}{}, ctx.Err()
		})
		if mode == "success" && err != nil || mode != "success" && !errors.Is(err, failure) {
			t.Fatalf("%s: %v", mode, err)
		}
	}
	after, err := os.Stat("/proc/self/ns/net")
	if err != nil || !os.SameFile(before, after) {
		t.Fatalf("parent namespace changed: %v", err)
	}
	// A named namespace can be re-entered by a different, disposable thread.
	isolated(t, func(ctx context.Context) {
		saved := filepath.Join("/run", "system-test-netns")
		file, err := os.Create(saved)
		if err != nil {
			t.Fatal(err)
		}
		if err := file.Close(); err != nil {
			t.Fatal(err)
		}
		if err := system.Bind(ctx, "/proc/thread-self/ns/net", saved); err != nil {
			t.Fatal(err)
		}
		defer unmount(ctx, t, saved)
		expected, err := os.Stat(saved)
		if err != nil {
			t.Fatal(err)
		}
		// A new goroutine begins in the process's mount namespace, so pass the
		// calling thread's namespace path rather than its private /run mount.
		link, err := os.Readlink("/proc/thread-self")
		if err != nil {
			t.Fatal(err)
		}
		_, err = system.RunNamespace(ctx, system.Network(filepath.Join("/proc", link, "ns/net")), func(context.Context) (struct{}, error) {
			actual, err := os.Stat("/proc/thread-self/ns/net")
			if err != nil {
				return struct{}{}, err
			}
			if !os.SameFile(expected, actual) {
				return struct{}{}, errors.New("entered wrong named namespace")
			}
			return struct{}{}, nil
		})
		if err != nil {
			t.Fatal(err)
		}
	})
	_, err = system.RunNamespace(t.Context(), system.HostMount(), func(context.Context) (struct{}, error) {
		actual, err := os.Stat("/proc/thread-self/ns/mnt")
		if err != nil {
			return struct{}{}, err
		}
		expected, err := os.Stat("/proc/1/ns/mnt")
		if err != nil {
			return struct{}{}, err
		}
		if !os.SameFile(expected, actual) {
			return struct{}{}, errors.New("entered wrong host mount namespace")
		}
		return struct{}{}, nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestNamespaceCancellationJoinsJob(t *testing.T) {
	if os.Geteuid() != 0 {
		t.Skip("needs explicit root invocation")
	}
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	started := make(chan struct{})
	finished := make(chan struct{})
	done := make(chan error, 1)
	go func() {
		_, err := system.RunNamespace(ctx, system.NewNetwork(), func(ctx context.Context) (struct{}, error) {
			close(started)
			<-ctx.Done()
			close(finished)
			return struct{}{}, ctx.Err()
		})
		done <- err
	}()
	select {
	case <-started:
	case err := <-done:
		t.Fatalf("namespace entry: %v", err)
	}
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	select {
	case <-finished:
	default:
		t.Fatal("namespace returned before job completed")
	}
}

func TestIsolationFixtureKeepsRunPrivate(t *testing.T) {
	if os.Geteuid() != 0 {
		t.Skip("needs explicit root invocation")
	}
	before, err := os.Stat("/run")
	if err != nil {
		t.Fatal(err)
	}
	if err := systemtest.Isolate(t.Context(), func(context.Context) error {
		during, err := os.Stat("/run")
		if err != nil {
			return err
		}
		if os.SameFile(before, during) {
			return errors.New("fixture used host /run")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	after, err := os.Stat("/run")
	if err != nil || !os.SameFile(before, after) {
		t.Fatalf("fixture changed host /run: %v", err)
	}
}

func TestSavedMountNamespaceUsesBorrowedDescriptor(t *testing.T) {
	if os.Geteuid() != 0 {
		t.Skip("needs explicit root invocation")
	}
	var saved *os.File
	defer func() {
		if saved != nil {
			if err := saved.Close(); err != nil {
				t.Error(err)
			}
		}
	}()
	const marker = "/run/demi-system-descriptor-marker"
	if err := systemtest.Isolate(t.Context(), func(context.Context) error {
		if err := os.WriteFile(marker, []byte("saved namespace"), 0600); err != nil {
			return err
		}
		const path = "/run/demi-system-saved-mount"
		// A temporary link gives the descriptor a name that can be removed.
		// Binding a mount namespace inside itself would create a kernel-rejected cycle.
		if err := os.Symlink("/proc/thread-self/ns/mnt", path); err != nil {
			return err
		}
		var err error
		saved, err = os.Open(path)
		removeErr := os.Remove(path)
		return errors.Join(err, removeErr)
	}); err != nil {
		t.Fatal(err)
	}
	for range 2 {
		data, err := system.RunNamespace(t.Context(), system.Mount(saved), func(context.Context) ([]byte, error) {
			return os.ReadFile(marker)
		})
		if err != nil || string(data) != "saved namespace" {
			t.Fatalf("saved mount read = %q, %v", data, err)
		}
		if _, err := saved.Stat(); err != nil {
			t.Fatalf("RunNamespace closed borrowed descriptor: %v", err)
		}
	}
}
