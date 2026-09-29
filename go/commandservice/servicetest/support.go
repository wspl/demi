// Package servicetest supports tests of command services: a service served over
// an in-memory connection or started as a child process, and a numbers source
// that counts.
package servicetest

import (
	"bytes"
	"context"
	"net"
	"os"
	"os/exec"
	"sync"
	"testing"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/internal/pipeconn"
)

// A Server is a service served over an in-memory connection, and the client
// that drives it.
type Server struct {
	Client *commandservice.Client

	stop   context.CancelFunc
	done   chan struct{}
	served error
}

// Start serves handler over an in-memory connection and connects a client to
// it. The test's cleanup closes the client, stops the service and waits for it.
func Start(t testing.TB, handler commandservice.Handler) *Server {
	t.Helper()
	ctx, stop := context.WithCancel(context.Background())
	clientEnd, serviceEnd := net.Pipe()
	server := &Server{stop: stop, done: make(chan struct{})}
	go func() {
		server.served = commandservice.Serve(ctx, serviceEnd, handler)
		close(server.done)
	}()
	client, err := commandservice.Connect(ctx, clientEnd)
	if err != nil {
		stop()
		t.Fatal(err)
	}
	server.Client = client
	t.Cleanup(func() {
		// A close that fails leaves nothing to release: the test is over.
		_ = client.Close()
		stop()
		<-server.done
	})
	return server
}

// Stop ends the context of [commandservice.Serve], as its owner does when the
// process is asked to stop.
func (s *Server) Stop() { s.stop() }

// Wait returns how [commandservice.Serve] returned, once it has, or the error
// of ctx when ctx ends first. A service ends when its client shuts it down or
// closes the connection, when its context ends, or when a fault retires it.
func (s *Server) Wait(ctx context.Context) error {
	select {
	case <-s.done:
		return s.served
	case <-ctx.Done():
		return ctx.Err()
	}
}

// A Process is a command service started as a child process, and the client
// that drives it over its standard input and output. Its standard error is
// kept, and the test logs it when it fails.
type Process struct {
	Client *commandservice.Client

	input  *os.File
	cmd    *exec.Cmd
	stderr lockedBuffer
	exited chan struct{}
	state  *os.ProcessState
}

// lockedBuffer is a buffer that a process writes while a test reads it.
type lockedBuffer struct {
	mu   sync.Mutex
	data bytes.Buffer
}

func (b *lockedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.data.Write(p)
}

func (b *lockedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.data.String()
}

// StartProcess starts program with args, and env beside the test's
// environment, and connects a client to it. The test's cleanup closes the
// client and kills the process if it still runs.
func StartProcess(ctx context.Context, t testing.TB, program string, args []string, env []string) *Process {
	t.Helper()
	toChild, childInput, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	childOutput, fromChild, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(program, args...)
	cmd.Env = append(os.Environ(), env...)
	cmd.Stdin = toChild
	cmd.Stdout = fromChild
	process := &Process{cmd: cmd, exited: make(chan struct{})}
	cmd.Stderr = &process.stderr
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	// The child holds its ends of the pipes now; closing ours cannot fail in a
	// way the test could act on.
	_ = toChild.Close()
	_ = fromChild.Close()
	go func() {
		// Wait's error only repeats the exit status the state holds.
		_ = cmd.Wait()
		process.state = cmd.ProcessState
		close(process.exited)
	}()
	process.input = childInput
	client, err := commandservice.Connect(ctx, pipeconn.New(childOutput, childInput))
	if err != nil {
		// The process may have exited already, which is what a kill that
		// fails says.
		_ = cmd.Process.Kill()
		<-process.exited
		t.Fatal(err)
	}
	process.Client = client
	t.Cleanup(func() {
		// Closing the client and killing a process that exited already both
		// fail harmlessly.
		_ = client.Close()
		_ = cmd.Process.Kill()
		<-process.exited
		if t.Failed() {
			t.Logf("standard error of %s:\n%s", program, process.stderr.String())
		}
	})
	return process
}

// Stderr returns what the process has written to its standard error so far.
func (p *Process) Stderr() string { return p.stderr.String() }

// CloseInput closes the process's standard input, which is the peer closing its
// side of the connection: the service reads the end of its input while the
// client can still read what the service writes.
func (p *Process) CloseInput() error { return p.input.Close() }

// Wait waits for the process to exit, and returns how it ended.
func (p *Process) Wait(ctx context.Context) (*os.ProcessState, error) {
	select {
	case <-p.exited:
		return p.state, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// counters give every conversation's sequences numbers that start at 1, as the
// backend's sequences do for a conversation that is new.
type counters struct {
	mu   sync.Mutex
	next map[string]uint64
}

// reserve returns the first of the next numbers of the request's conversation
// and sequence.
func (c *counters) reserve(_ context.Context, request commandservice.NumbersRequest) (uint64, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.next == nil {
		c.next = map[string]uint64{}
	}
	key := request.Conversation + "/" + string(request.Sequence)
	first := max(c.next[key], 1)
	c.next[key] = first + uint64(request.Count)
	return first, nil
}

// AnswerNumbers opens the numbers stream of the service client drives and
// answers it from counters, as a runner answers it from the backend's
// sequences, until the service ends it or ctx does. The channel receives how
// the answering ended.
func AnswerNumbers(ctx context.Context, client *commandservice.Client) (<-chan error, error) {
	stream, err := client.Numbers(ctx)
	if err != nil {
		return nil, err
	}
	answered := make(chan error, 1)
	go func() { answered <- stream.Answer(ctx, (&counters{}).reserve) }()
	return answered, nil
}
