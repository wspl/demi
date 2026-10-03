package process

import (
	"context"
	"errors"
	"os"
	"runtime"
	"sync"

	"golang.org/x/sys/windows"
)

// cancellableStdio owns synchronous Windows standard handles as well as pipes.
// Console reads cannot be interrupted by CancelIoEx alone: each active call
// keeps its OS thread until Close has cancelled and joined that call.
func cancellableStdio(ctx context.Context, stdio Stdio) (Stdio, func() error, error) {
	if err := ctx.Err(); err != nil {
		return stdio, func() error { return nil }, err
	}
	if err := cancelSynchronousIO.Find(); err != nil {
		return stdio, func() error { return nil }, err
	}
	files := map[*os.File]*synchronousCommandFile{}
	wrap := func(file *os.File) *synchronousCommandFile {
		if existing := files[file]; existing != nil {
			return existing
		}
		wrapped := &synchronousCommandFile{file: file, active: map[*commandFileCall]struct{}{}}
		files[file] = wrapped
		return wrapped
	}
	if file, ok := stdio.Stdin.(*os.File); ok {
		stdio.Stdin = wrap(file)
	}
	if file, ok := stdio.Stdout.(*os.File); ok {
		stdio.Stdout = wrap(file)
	}
	if file, ok := stdio.Stderr.(*os.File); ok {
		stdio.Stderr = wrap(file)
	}
	return stdio, func() error { return nil }, nil
}

// x/sys does not wrap CancelSynchronousIo. Microsoft documents a thread handle
// with THREAD_TERMINATE access and a BOOL result; ERROR_NOT_FOUND means the
// call has not entered the kernel yet or has already completed.
var cancelSynchronousIO = windows.NewLazySystemDLL("kernel32.dll").NewProc("CancelSynchronousIo")

type commandFileCall struct {
	thread        windows.Handle
	done, release chan struct{}
}
type synchronousCommandFile struct {
	file   *os.File
	mu     sync.Mutex
	active map[*commandFileCall]struct{}
	closed bool
	once   sync.Once
	err    error
}

func (f *synchronousCommandFile) Read(b []byte) (int, error) {
	return f.call(func() (int, error) { return f.file.Read(b) })
}

func (f *synchronousCommandFile) Write(b []byte) (int, error) {
	return f.call(func() (int, error) { return f.file.Write(b) })
}

// call reserves the OS thread through cancellation acknowledgement, preventing
// a late CancelSynchronousIo from cancelling unrelated work on a reused thread.
func (f *synchronousCommandFile) call(operation func() (int, error)) (int, error) {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	thread, err := windows.OpenThread(windows.THREAD_TERMINATE, false, windows.GetCurrentThreadId())
	if err != nil {
		return 0, err
	}
	defer func() { _ = windows.CloseHandle(thread) }() // The IO/cancellation result already records failure.
	call := &commandFileCall{thread: thread, done: make(chan struct{}), release: make(chan struct{})}
	f.mu.Lock()
	if f.closed {
		f.mu.Unlock()
		return 0, os.ErrClosed
	}
	f.active[call] = struct{}{}
	f.mu.Unlock()
	n, err := operation()
	f.mu.Lock()
	delete(f.active, call)
	closing := f.closed
	close(call.done)
	f.mu.Unlock()
	if closing {
		<-call.release
	}
	return n, err
}

func (f *synchronousCommandFile) Close() error {
	f.once.Do(func() {
		f.mu.Lock()
		f.closed = true
		calls := make([]*commandFileCall, 0, len(f.active))
		for call := range f.active {
			calls = append(calls, call)
		}
		f.mu.Unlock()
		// Cancel every active call before joining; two callers may be serialized
		// inside os.File. Recheck until the registered call enters/completes IO.
		for len(calls) > 0 {
			pending := calls[:0]
			for _, call := range calls {
				select {
				case <-call.done:
					close(call.release)
				default:
					result, _, err := cancelSynchronousIO.Call(uintptr(call.thread))
					if result == 0 && !errors.Is(err, windows.ERROR_NOT_FOUND) && f.err == nil {
						f.err = errors.Join(f.err, err)
					}
					pending = append(pending, call)
				}
			}
			calls = pending
			runtime.Gosched()
		}
		f.err = errors.Join(f.err, f.file.Close())
	})
	return f.err
}
