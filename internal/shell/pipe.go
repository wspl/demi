package shell

import (
	"context"
	"io"
	"os"
	"sync"
	"syscall"
	"time"
)

// pipe connects the writers and readers of one stream of a job: a pipeline
// stage, a here-document, or the job's standard input or output. While only
// in-process commands use it, bytes pass in memory: a write waits until
// readers have taken all of it, so nothing is consumed that no command asked
// for. Once an external program takes either end, the pipe becomes an OS pipe
// for both ends, and the program uses it directly.
//
// Closing the reader makes writes fail with EPIPE; closing the writer gives
// readers the end of file once they have taken everything written.
type pipe struct {
	// wrMu makes each write reach readers whole, before another writer's.
	wrMu sync.Mutex
	// rdMu lets one read at a time wait for data.
	rdMu sync.Mutex

	mu sync.Mutex
	// changed is closed and replaced whenever the fields below change.
	changed chan struct{}
	// pending is the part of the current write that readers have not taken.
	pending []byte
	// readerClosed and writerClosed record which ends were closed.
	readerClosed bool
	writerClosed bool
	// interrupted makes waiting reads fail, as a past read deadline does.
	interrupted bool
	// drained is set once reading the OS pipe reached its end of file,
	// after which the reading end is closed and reads report the end again.
	drained bool
	// r and w are the OS pipe once an external program took an end.
	r *os.File
	w *os.File
	// closed, when set, is called once both ends are closed.
	closed func()
}

// newPipe returns the two ends of a new pipe.
func newPipe() (*pipeReader, *pipeWriter) {
	p := &pipe{changed: make(chan struct{})}
	return &pipeReader{p}, &pipeWriter{p}
}

// notify wakes everyone waiting for a change. It is called with mu held.
func (p *pipe) notify() {
	close(p.changed)
	p.changed = make(chan struct{})
}

// wait releases mu until the next change and takes it again.
func (p *pipe) wait() {
	changed := p.changed
	p.mu.Unlock()
	<-changed
	p.mu.Lock()
}

func (p *pipe) read(b []byte) (int, error) {
	p.rdMu.Lock()
	defer p.rdMu.Unlock()
	p.mu.Lock()
	for {
		switch {
		case p.drained:
			p.mu.Unlock()
			return 0, io.EOF
		case p.readerClosed:
			p.mu.Unlock()
			return 0, os.ErrClosed
		case p.r != nil:
			file := p.r
			p.mu.Unlock()
			n, err := file.Read(b)
			if err == io.EOF {
				// Every writer is gone for good: release the descriptor.
				p.mu.Lock()
				p.drained = true
				p.mu.Unlock()
				p.closeReader()
			}
			return n, err
		case p.interrupted:
			p.mu.Unlock()
			return 0, os.ErrDeadlineExceeded
		case len(p.pending) > 0:
			n := copy(b, p.pending)
			p.pending = p.pending[n:]
			p.notify()
			p.mu.Unlock()
			return n, nil
		case p.writerClosed:
			p.mu.Unlock()
			return 0, io.EOF
		}
		p.wait()
	}
}

func (p *pipe) write(b []byte) (int, error) {
	p.wrMu.Lock()
	defer p.wrMu.Unlock()
	p.mu.Lock()
	p.pending = b
	p.notify()
	for {
		taken := len(b) - len(p.pending)
		switch {
		case p.writerClosed:
			p.pending = nil
			p.mu.Unlock()
			return taken, os.ErrClosed
		case p.w != nil:
			// An external program took an end: the rest goes to the OS pipe,
			// where an external reader may still take it.
			rest := p.pending
			p.pending = nil
			file := p.w
			p.mu.Unlock()
			written, err := file.Write(rest)
			return taken + written, err
		case p.readerClosed:
			p.pending = nil
			p.mu.Unlock()
			return taken, syscall.EPIPE
		case len(p.pending) == 0:
			p.mu.Unlock()
			return len(b), nil
		}
		p.wait()
	}
}

// closeReader closes the reading end.
func (p *pipe) closeReader() error {
	p.mu.Lock()
	if p.readerClosed {
		p.mu.Unlock()
		return nil
	}
	p.readerClosed = true
	p.notify()
	var err error
	if p.r != nil {
		err = p.r.Close()
	}
	p.closedEnd()
	return err
}

// closeWriter closes the writing end.
func (p *pipe) closeWriter() error {
	p.mu.Lock()
	if p.writerClosed {
		p.mu.Unlock()
		return nil
	}
	p.writerClosed = true
	p.notify()
	var err error
	if p.w != nil {
		err = p.w.Close()
	}
	p.closedEnd()
	return err
}

// closedEnd releases mu after an end was closed, and reports when both are.
func (p *pipe) closedEnd() {
	both := p.readerClosed && p.writerClosed
	p.mu.Unlock()
	if both && p.closed != nil {
		p.closed()
	}
}

// files turns the pipe into an OS pipe, if it is not one yet, and returns
// its ends for an external program. An end already closed here is closed in
// the OS pipe too, so the program sees the end of file or EPIPE.
func (p *pipe) files(ctx context.Context) (*os.File, *os.File, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.r != nil {
		return p.r, p.w, nil
	}
	r, w, err := osPipe(ctx)
	if err != nil {
		return nil, nil, err
	}
	// Closing an end that nothing else uses cannot fail in a way that
	// matters: the descriptor is released either way.
	if p.readerClosed {
		r.Close()
	}
	if p.writerClosed {
		w.Close()
	}
	p.r, p.w = r, w
	p.notify()
	return r, w, nil
}

// setReadDeadline supports the read builtin, which interrupts a waiting read
// with a deadline in the past when its context ends.
func (p *pipe) setReadDeadline(t time.Time) error {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.r != nil {
		return p.r.SetReadDeadline(t)
	}
	p.interrupted = !t.IsZero() && !t.After(time.Now())
	p.notify()
	return nil
}

// pipeReader is the reading end of a pipe.
type pipeReader struct{ p *pipe }

func (r *pipeReader) Read(b []byte) (int, error) { return r.p.read(b) }

// Close closes the reading end; writes then fail with EPIPE.
func (r *pipeReader) Close() error { return r.p.closeReader() }

// SetReadDeadline interrupts waiting reads when t is in the past.
func (r *pipeReader) SetReadDeadline(t time.Time) error { return r.p.setReadDeadline(t) }

// file returns the reading end as a file for an external program.
func (r *pipeReader) file(ctx context.Context) (*os.File, error) {
	file, _, err := r.p.files(ctx)
	return file, err
}

// pipeWriter is the writing end of a pipe.
type pipeWriter struct{ p *pipe }

func (w *pipeWriter) Write(b []byte) (int, error) { return w.p.write(b) }

// Close closes the writing end; readers then reach the end of file.
func (w *pipeWriter) Close() error { return w.p.closeWriter() }

// file returns the writing end as a file for an external program.
func (w *pipeWriter) file(ctx context.Context) (*os.File, error) {
	_, file, err := w.p.files(ctx)
	return file, err
}
