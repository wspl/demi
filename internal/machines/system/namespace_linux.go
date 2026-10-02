//go:build linux

package system

import (
	"context"
	"fmt"
	"os"
	"runtime"

	"golang.org/x/sys/unix"
)

// Namespace selects the namespace a job runs in. Use HostMount, Mount, Network or NewNetwork.
type Namespace struct {
	kind namespaceKind
	path string
	file *os.File
}

type namespaceKind uint8

const (
	hostMountKind namespaceKind = iota + 1
	networkKind
	newNetworkKind
	savedMountKind
)

// HostMount selects PID 1's mount namespace, where namespace handles live.
func HostMount() Namespace { return Namespace{kind: hostMountKind, path: "/proc/1/ns/mnt"} }

// Mount selects an already opened saved mount namespace. RunNamespace borrows
// the descriptor without reopening its path or closing it. The caller must
// keep file open until RunNamespace returns.
func Mount(file *os.File) Namespace { return Namespace{kind: savedMountKind, file: file} }

// Network selects the network namespace bound at path, such as /run/netns/demi-3.
func Network(path string) Namespace { return Namespace{kind: networkKind, path: path} }

// NewNetwork selects a new network namespace, which the job may bind to a path.
func NewNetwork() Namespace { return Namespace{kind: newNetworkKind} }

// RunNamespace runs the whole job in an owned goroutine that locks its OS
// thread, unshares CLONE_FS, enters namespace and exits without unlocking.
// It waits for the job even on cancellation. The job must close namespace-bound
// resources before returning and must do namespace work on that goroutine.
// A recovery child must be started and waited for on that same goroutine.
// Panics from the job are recovered and returned as errors.
func RunNamespace[T any](ctx context.Context, namespace Namespace, job func(context.Context) (T, error)) (T, error) {
	var value T
	var err error
	done := make(chan struct{})
	go func() {
		runtime.LockOSThread()
		// Never unlock: even a failed namespace entry must retire this thread.
		defer close(done)
		defer func() {
			if failure := recover(); failure != nil {
				if cause, ok := failure.(error); ok {
					err = fmt.Errorf("namespace job panicked: %w", cause)
				} else {
					err = fmt.Errorf("namespace job panicked: %v", failure)
				}
			}
		}()
		if err = ctx.Err(); err != nil {
			return
		}
		if err = unix.Unshare(unix.CLONE_FS); err != nil {
			return
		}
		switch namespace.kind {
		case savedMountKind:
			err = unix.Setns(int(namespace.file.Fd()), unix.CLONE_NEWNS)
			runtime.KeepAlive(namespace.file)
		case newNetworkKind:
			err = unix.Unshare(unix.CLONE_NEWNET)
		default:
			var file *os.File
			file, err = os.Open(namespace.path)
			if err != nil {
				return
			}
			// The namespace descriptor is only read; setns retains the namespace.
			defer func() { _ = file.Close() }()
			kind := unix.CLONE_NEWNET
			if namespace.kind == hostMountKind {
				kind = unix.CLONE_NEWNS
			}
			err = unix.Setns(int(file.Fd()), kind)
		}
		if err != nil {
			return
		}
		value, err = job(ctx)
	}()
	<-done
	return value, err
}
