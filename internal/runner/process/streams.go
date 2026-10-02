package process

import (
	"context"
	"errors"
	"io"
	"os"
	"os/exec"
	"reflect"
	"sync"
	"syscall"

	"github.com/wspl/demi/internal/cmdsdk"
)

// commandStreams keeps exec.Cmd free of hidden IO goroutines: the leader is
// reaped first, then its group is killed and these owned copies are joined.
type commandStreams struct {
	copies    []streamCopy
	childEnds []*os.File
	workers   sync.WaitGroup
	mu        sync.Mutex
	err       error
	cancel    context.CancelFunc
}
type streamCopy struct {
	pipe          *os.File
	source        io.Reader
	target        io.Writer
	input         bool
	closeExternal func()
}

// prepareStreams connects non-file standard IO through cancellable owned pipes.
// Callers supplying blocking IO must provide Close that interrupts that IO.
func prepareStreams(ctx context.Context, cmd *exec.Cmd) (*commandStreams, error) {
	s := &commandStreams{}
	sharedOutput := cmd.Stdout != nil && reflect.TypeOf(cmd.Stdout).Comparable() && cmd.Stdout == cmd.Stderr
	add := func(source io.Reader, target io.Writer, input bool) (*os.File, error) {
		pair, err := cmdsdk.Retry(ctx, func() ([2]*os.File, error) {
			r, w, err := os.Pipe()
			return [2]*os.File{r, w}, err
		})
		if err != nil {
			return nil, err
		}
		r, w := pair[0], pair[1]
		child, parent := w, r
		if input {
			child, parent = r, w
			target = w
		} else {
			source = r
		}
		s.childEnds = append(s.childEnds, child)
		stream := streamCopy{pipe: parent, source: source, target: target, input: input}
		var external io.Closer
		if input {
			external, _ = source.(io.Closer)
		} else {
			external, _ = target.(io.Closer)
		}
		stream.closeExternal = sync.OnceFunc(func() {
			if external != nil {
				_ = external.Close()
			}
		}) // Cancellation or leader exit already determines the outcome.
		s.copies = append(s.copies, stream)
		return child, nil
	}
	var err error
	if cmd.Stdin != nil {
		if _, file := cmd.Stdin.(*os.File); !file {
			cmd.Stdin, err = add(cmd.Stdin, nil, true)
		}
	}
	if err == nil && cmd.Stdout != nil {
		if _, file := cmd.Stdout.(*os.File); !file {
			cmd.Stdout, err = add(nil, cmd.Stdout, false)
		}
	}
	if sharedOutput {
		cmd.Stderr = cmd.Stdout
	}
	if err == nil && cmd.Stderr != nil && !sharedOutput {
		if _, file := cmd.Stderr.(*os.File); !file {
			cmd.Stderr, err = add(nil, cmd.Stderr, false)
		}
	}
	if err != nil {
		s.close()
		return nil, err
	}
	return s, nil
}
func (s *commandStreams) start() {
	for _, file := range s.childEnds {
		_ = file.Close()
	} // Only child copies remain in use.
	s.childEnds = nil
	for _, stream := range s.copies {
		s.workers.Add(1)
		go func() {
			defer s.workers.Done()
			defer func() { _ = stream.pipe.Close() }() // Closing an already interrupted pipe is harmless.
			_, err := io.CopyBuffer(stream.target, stream.source, make([]byte, 64*1024))
			if stream.input && errors.Is(err, syscall.EPIPE) {
				return
			}
			s.record(err)
		}()
	}
}
func (s *commandStreams) record(err error) {
	if err == nil || errors.Is(err, os.ErrClosed) || errors.Is(err, io.ErrClosedPipe) || errors.Is(err, context.Canceled) {
		return
	}
	s.mu.Lock()
	s.err = errors.Join(s.err, err)
	s.mu.Unlock()
	if s.cancel != nil {
		s.cancel()
	}
}
func (s *commandStreams) failure() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.err
}
func (s *commandStreams) stopInput() {
	for _, stream := range s.copies {
		if stream.input {
			_ = stream.pipe.Close() // Unblock the copy after the child stops accepting input.
			stream.closeExternal()
		}
	}
}
func (s *commandStreams) interrupt() {
	for _, stream := range s.copies {
		_ = stream.pipe.Close() // Cancellation supersedes close failures.
		stream.closeExternal()
	}
}
func (s *commandStreams) close() {
	for _, file := range s.childEnds {
		_ = file.Close()
	} // Failed start cleanup.
	s.interrupt()
}
func (s *commandStreams) wait(ctx context.Context) {
	if ctx.Err() != nil {
		s.interrupt()
	}
	s.workers.Wait()
}
