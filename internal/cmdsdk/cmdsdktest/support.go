package cmdsdktest

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// Pauses is the process-wide count of descriptor retry pauses.
func Pauses() uint64 { return cmdsdk.DescriptorPauses() }

type counters struct {
	mu   sync.Mutex
	next map[string]uint64
}

func (c *counters) take(q commandwire.NumbersRequest) uint64 {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.next == nil {
		c.next = map[string]uint64{}
	}
	key := q.Conversation + ":" + string(q.Sequence)
	first := c.next[key]
	if first == 0 {
		first = 1
	}
	c.next[key] = first + uint64(q.Count)
	return first
}

// CountingNumbers supplies per-conversation sequences from 1, with test-owned cleanup.
func CountingNumbers(t testing.TB) *cmdsdk.Numbers {
	t.Helper()
	n, requests := cmdsdk.NumbersChannel()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		defer close(done)
		var c counters
		for {
			select {
			case <-ctx.Done():
				return
			case p := <-requests:
				p.Reply(c.take(p.Request), nil)
			}
		}
	}()
	t.Cleanup(func() {
		cancel()
		n.Close()
		<-done
	})
	return n
}

// ArtifactsFrom supplies artifact answers to local handlers with test-owned cleanup.
func ArtifactsFrom(t testing.TB, answer func(context.Context, commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error)) *cmdsdk.Artifacts {
	t.Helper()
	a, requests := cmdsdk.ArtifactsChannel()
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		defer close(done)
		for {
			select {
			case <-ctx.Done():
				return
			case p := <-requests:
				r, err := answer(ctx, p.Request)
				p.Reply(r, err)
			}
		}
	}()
	t.Cleanup(func() {
		cancel()
		a.Close()
		<-done
	})
	return a
}

// AnswerNumbers opens and answers a service stream until shutdown or context cancellation.
func AnswerNumbers(ctx context.Context, client *cmdsdk.Client) error {
	s, err := client.Numbers(ctx)
	if err != nil {
		return err
	}
	var c counters
	return s.AnswerNumbers(ctx, func(_ context.Context, q commandwire.NumbersRequest) (uint64, error) { return c.take(q), nil })
}

// AnswerArtifacts opens and answers the service's artifact stream.
func AnswerArtifacts(ctx context.Context, client *cmdsdk.Client, answer func(context.Context, commandwire.ArtifactRequest) (commandwire.ArtifactAnswer, error)) error {
	s, err := client.Artifacts(ctx)
	if err != nil {
		return err
	}
	return s.AnswerArtifacts(ctx, answer)
}

// ServiceProcess is a test-owned child service and its protocol client.
type ServiceProcess struct {
	Client  *cmdsdk.Client
	command *exec.Cmd
	wait    chan error
	once    sync.Once
	err     error
}

// Start starts a supplied program path (normally resolved by programtest), inheriting stderr.
func Start(ctx context.Context, t testing.TB, program string, args, env []string) (*ServiceProcess, error) {
	t.Helper()
	input, writer, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	reader, output, err := os.Pipe()
	if err != nil {
		return nil, errors.Join(err, input.Close(), writer.Close())
	}
	command := exec.CommandContext(ctx, program, args...)
	command.Env = append(os.Environ(), env...)
	command.Stdin = input
	command.Stdout = output
	command.Stderr = os.Stderr
	if err = command.Start(); err != nil {
		return nil, errors.Join(err, input.Close(), writer.Close(), reader.Close(), output.Close())
	}
	// The child owns copies of these pipe ends.
	_ = input.Close()
	_ = output.Close()
	process := &ServiceProcess{command: command, wait: make(chan error, 1)}
	go func() { process.wait <- command.Wait() }()
	client, err := cmdsdk.Connect(ctx, &cmdsdk.PipeConn{Reader: reader, Writer: writer})
	if err != nil {
		_ = command.Process.Kill()
		<-process.wait
		return nil, err
	}
	process.Client = client
	t.Cleanup(func() { _ = process.Close() })
	return process, nil
}

// PID identifies the running service.
func (p *ServiceProcess) PID() int { return p.command.Process.Pid }

// Close stops and reaps the service; it is idempotent.
func (p *ServiceProcess) Close() error {
	p.once.Do(func() {
		_ = p.Client.Close()
		_ = p.command.Process.Kill()
		p.err = <-p.wait
	})
	return p.err
}

// Shutdown requests graceful shutdown and reaps the child.
func (p *ServiceProcess) Shutdown(ctx context.Context) error {
	if err := p.Client.Shutdown(ctx); err != nil {
		return err
	}
	p.once.Do(func() {
		select {
		case p.err = <-p.wait:
		case <-ctx.Done():
			_ = p.command.Process.Kill()
			p.err = errors.Join(ctx.Err(), <-p.wait)
		}
		p.err = errors.Join(p.err, p.Client.Close())
	})
	return p.err
}
