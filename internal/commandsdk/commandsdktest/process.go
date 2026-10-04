package commandsdktest

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/commandsdk"
)

// ServiceProcess is a test-owned child service and its protocol client.
type ServiceProcess struct {
	Client  *commandsdk.Client
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
	go func() {
		process.wait <- command.Wait()
	}()
	client, err := commandsdk.Connect(ctx, &commandsdk.PipeConn{Reader: reader, Writer: writer})
	if err != nil {
		_ = command.Process.Kill()
		<-process.wait
		return nil, err
	}
	process.Client = client
	t.Cleanup(func() {
		_ = process.Close()
	})
	return process, nil
}

// PID identifies the running service.
func (p *ServiceProcess) PID() int {
	return p.command.Process.Pid
}

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
