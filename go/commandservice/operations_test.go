package commandservice_test

import (
	"io"
	"maps"
	"slices"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/commandservice"
)

// operations is a handler whose operations are functions, and which holds no
// conversation state.
type operations map[string]func(call *commandservice.Call) (commandservice.Completion, error)

func (o operations) Operations() []string {
	return slices.Sorted(maps.Keys(o))
}

func (o operations) Invoke(call *commandservice.Call) (commandservice.Completion, error) {
	return o[call.Invocation.Operation](call)
}

// signal is an event a handler tells a test about. Its capacity is large so
// that a handler never waits for a test that has stopped listening.
type signal chan struct{}

func newSignal() signal { return make(signal, 1024) }

func (s signal) send() { s <- struct{}{} }

// receive waits for the event, and fails the test if it does not come; the
// wait is a guard against a hang, not a window the event must fall into.
func (s signal) receive(t testing.TB) {
	t.Helper()
	select {
	case <-s:
	case <-time.After(hang):
		t.Fatal("the event did not come")
	}
}

// echo writes each chunk of input back, then "done" to standard error, and
// exits 7.
func echo(call *commandservice.Call) (commandservice.Completion, error) {
	for {
		chunk, err := call.Stdin.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return commandservice.Completion{}, err
		}
		if _, err := call.Stdout.Write(chunk); err != nil {
			return commandservice.Completion{}, err
		}
	}
	_, err := io.WriteString(call.Stderr, "done")
	return commandservice.Completion{ExitCode: 7}, err
}

// hold reads its input until the caller ends it, as a browser wait does.
func hold(call *commandservice.Call) (commandservice.Completion, error) {
	for {
		if _, err := call.Stdin.Next(); err != nil {
			if err == io.EOF {
				return commandservice.Completion{}, nil
			}
			return commandservice.Completion{}, err
		}
	}
}

// flood writes 64 KiB records until its output fails, which it tells `ended`.
func flood(ended signal) func(*commandservice.Call) (commandservice.Completion, error) {
	return func(call *commandservice.Call) (commandservice.Completion, error) {
		record := make([]byte, 64*1024)
		for {
			if _, err := call.Stdout.Write(record); err != nil {
				ended.send()
				return commandservice.Completion{}, err
			}
		}
	}
}

// short writes "ok" and completes.
func short(call *commandservice.Call) (commandservice.Completion, error) {
	_, err := io.WriteString(call.Stdout, "ok")
	return commandservice.Completion{}, err
}

// events records what happened, in order, from any goroutine.
type events struct {
	mu   sync.Mutex
	list []string
}

func (e *events) add(event string) {
	e.mu.Lock()
	defer e.mu.Unlock()
	e.list = append(e.list, event)
}

func (e *events) all() []string {
	e.mu.Lock()
	defer e.mu.Unlock()
	return slices.Clone(e.list)
}
