package engine

import (
	"context"
	"io"
	"os"
	"os/exec"
	"time"
)

// interruptStreams wakes shell builtins blocked on the invocation's borrowed
// pipes without closing the caller's handles. Regular files do not need deadlines.
func interruptStreams(ctx context.Context, options Options) func() {
	done := make(chan struct{})
	var readers, writers []*os.File
	if options.Stdin != nil {
		readers = append(readers, options.Stdin)
	}
	for _, output := range []any{options.Stdout, options.Stderr} {
		if file, ok := output.(*os.File); ok && file != nil {
			writers = append(writers, file)
		}
	}
	set := func(deadline time.Time) {
		for _, file := range readers {
			_ = file.SetReadDeadline(deadline)
		}
		for _, file := range writers {
			_ = file.SetWriteDeadline(deadline)
		}
	} // Regular files and already closed handles may reject deadlines harmlessly.
	stop := context.AfterFunc(ctx, func() {
		set(time.Now())
		close(done)
	})
	return func() {
		if !stop() {
			<-done
			set(time.Time{})
		}
	}
}

// borrowStreams separates a command's cancellation from the shell's handles.
// Stdin stays inherited: copying it would consume input even when the child
// never reads. Polling is restored after the child has finished.
func borrowStreams(ctx context.Context, cmd *exec.Cmd) (func(), error) {
	borrowed := make(map[*os.File]io.ReadWriteCloser)
	closeFiles := func() {
		if input, ok := cmd.Stdin.(*os.File); ok && input != nil {
			restoreInputPolling(input)
		}
		for _, file := range borrowed {
			_ = file.Close()
		}
	}
	borrow := func(value any) (io.ReadWriteCloser, error) {
		file, ok := value.(*os.File)
		if !ok || file == nil {
			return nil, nil
		}
		if duplicate := borrowed[file]; duplicate != nil {
			return duplicate, nil
		}
		duplicate, err := borrowFile(ctx, file)
		if err != nil {
			return nil, err
		}
		borrowed[file] = duplicate
		return duplicate, nil
	}
	for index, stream := range []any{cmd.Stdout, cmd.Stderr} {
		file, err := borrow(stream)
		if err != nil {
			closeFiles()
			return nil, err
		}
		if file == nil {
			continue
		}
		switch index {
		case 0:
			cmd.Stdout = file
		case 1:
			cmd.Stderr = file
		}
	}
	return closeFiles, nil
}
