//go:build linux

package linux

import (
	"os"
	"runtime"

	"golang.org/x/sys/unix"
)

// init locks the main goroutine to the main thread for the life of the program.
// The main thread is the thread group leader, and /proc/<pid>/ns/* names the
// leader's namespaces: the manager binds /proc/<pid>/ns/mnt as its saved mount
// namespace, so the leader must never enter another. A goroutine locked to a
// thread that has entered a namespace never returns it to the pool, but the
// runtime does not end the main thread (it parks it), so a namespace job that
// ran on it would leave the whole process, as far as /proc is concerned, in the
// other namespace. With the main goroutine holding the main thread, no other
// goroutine runs on it.
func init() {
	runtime.LockOSThread()
}

// InNamespace runs job on a goroutine of its own, locked to its thread, after
// enter has moved the thread into another namespace, and returns the job's
// result. The goroutine ends without unlocking, so the runtime ends the thread
// with it: no later goroutine runs in the wrong namespace (a thread cannot
// return to the namespace of the others in every case, and must never be
// reused). A job that starts a child from this thread starts it inside the
// namespace.
func InNamespace(enter func() error, job func() error) error {
	done := make(chan error, 1)
	go func() {
		runtime.LockOSThread()
		// No UnlockOSThread: the thread ends with the goroutine.
		if err := enter(); err != nil {
			done <- err
			return
		}
		done <- job()
	}()
	return <-done
}

// InMountNamespace runs job in the mount namespace of the file at path, such as
// /proc/1/ns/mnt or a namespace handle. A thread that shares its filesystem
// context with others cannot enter a mount namespace, so it gets its own first.
func InMountNamespace(path string, job func() error) error {
	return InNamespace(func() error { return enterMount(path) }, job)
}

// InMountNamespaceFile is [InMountNamespace] for a namespace already open.
func InMountNamespaceFile(namespace *os.File, job func() error) error {
	return InNamespace(func() error { return enterMountFile(namespace) }, job)
}

// InNetworkNamespace runs job in the network namespace bound at path, such as a
// slot's /run/netns/demi-3.
func InNetworkNamespace(path string, job func() error) error {
	return InNamespace(func() error {
		namespace, err := os.Open(path)
		if err != nil {
			return err
		}
		defer namespace.Close()
		return unix.Setns(int(namespace.Fd()), unix.CLONE_NEWNET)
	}, job)
}

// InNewNetworkNamespace runs job in a new network namespace, which the job may
// bind to a path.
func InNewNetworkNamespace(job func() error) error {
	return InNamespace(func() error { return unix.Unshare(unix.CLONE_NEWNET) }, job)
}

func enterMount(path string) error {
	namespace, err := os.Open(path)
	if err != nil {
		return err
	}
	defer namespace.Close()
	return enterMountFile(namespace)
}

func enterMountFile(namespace *os.File) error {
	if err := unix.Unshare(unix.CLONE_FS); err != nil {
		return err
	}
	return unix.Setns(int(namespace.Fd()), unix.CLONE_NEWNS)
}

// OnThread runs job on a goroutine of its own, locked to its thread, and returns
// its result: work that must not share a thread with the runtime's other
// goroutines, such as the frozen window of a checkpoint. The thread is unlocked
// when the job returns; a panic ends the process after the job's deferred calls
// have run.
func OnThread(job func() error) error {
	done := make(chan error, 1)
	go func() {
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
		done <- job()
	}()
	return <-done
}
