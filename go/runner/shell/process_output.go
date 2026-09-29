package shell

import (
	"context"
	"errors"
	"io"
	"os"
	"sync"
)

// processOutput drains child pipes independently of exec.Cmd.Wait, so the
// process owner can reap the leader and kill descendants before joining I/O.
type processOutput struct {
	readers, writers []*os.File
	workers          sync.WaitGroup
	mu               sync.Mutex
	err              error
}

func (p *processOutput) connect(destination io.Writer) (io.Writer, error) {
	if file, ok := destination.(*os.File); ok {
		return file, nil
	}
	reader, writer, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	p.readers = append(p.readers, reader)
	p.writers = append(p.writers, writer)
	p.workers.Go(func() {
		defer reader.Close()
		_, err := io.Copy(destination, reader)
		if err != nil && !errors.Is(err, os.ErrClosed) {
			p.mu.Lock()
			p.err = errors.Join(p.err, err)
			p.mu.Unlock()
		}
	})
	return writer, nil
}
func (p *processOutput) closeWriters() {
	for _, file := range p.writers {
		file.Close()
	}
}
func (p *processOutput) finish(ctx context.Context) error {
	p.closeWriters()
	canceled := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		defer close(canceled)
		for _, file := range p.readers {
			file.Close()
		}
	})
	p.workers.Wait()
	if !stop() {
		<-canceled
	}
	for _, file := range p.readers {
		file.Close()
	}
	return p.err
}
