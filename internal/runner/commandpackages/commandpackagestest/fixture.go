package commandpackagestest

import (
	"context"
	"errors"
	"fmt"
	"io"
	"math"
	"os"
	"slices"
	"strconv"
	"sync"
	"sync/atomic"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandproto/commandprototest"
	"github.com/wspl/demi/internal/commandsdk"
)

// NoNumbers refuses requests as a backend does for a conversation not on the device.
type NoNumbers struct{}

// Reserve refuses every request with "this test gives out no conversation numbers".
func (NoNumbers) Reserve(context.Context, string, commandproto.ServiceSequence, uint32) (uint64, error) {
	return 0, errors.New("this test gives out no conversation numbers")
}

// Fixture supplies the deliberately faulty demi-native-fixture service operations.
// Its zero value is ready to pass to commandsdk.ServeStdio. Crash and spin operations
// must only run in the fixture child process, never in the test's own process.
type Fixture struct {
	mu               sync.Mutex
	conversations    map[string]bool
	stalling         map[string]bool
	stalled, proceed chan struct{}
	numbers          *commandsdk.Numbers
}

func (f *Fixture) initLocked() {
	if f.conversations == nil {
		f.conversations = make(map[string]bool)
		f.stalling = make(map[string]bool)
		f.stalled = make(chan struct{}, 1)
		f.proceed = make(chan struct{}, 1)
	}
}

// Operations returns the shared commandwiretest fixture operation catalog.
func (f *Fixture) Operations() []string {
	return commandprototest.FixtureOperations()
}

// SetNumbers supplies this connection's conversation number source.
func (f *Fixture) SetNumbers(numbers *commandsdk.Numbers) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.numbers = numbers
}

// Invoke runs a fixture operation with its supplied IO and cancellation.
func (f *Fixture) Invoke(
	ctx context.Context,
	invocation commandsdk.InvocationContext[commandproto.Invocation],
) (commandproto.Completion, error) {
	f.mu.Lock()
	f.initLocked()
	numbers, stalled, proceed := f.numbers, f.stalled, f.proceed
	f.mu.Unlock()
	result := commandproto.Completion{}
	request := invocation.Request
	switch request.Operation {
	case "number":
		return f.drawNumbers(ctx, invocation, numbers)
	case "stalled":
		select {
		case <-ctx.Done():
			return result, ctx.Err()
		case <-stalled:
		}
	case "proceed":
		select {
		case proceed <- struct{}{}:
		default:
		}
	case "retain", "stall_release":
		f.retain(request)
	case "held":
		return f.heldAnswer(ctx, invocation.Output)
	case "crash":
		_, _ = fmt.Fprintln(
			os.Stderr,
			"fixture crashing on purpose",
		) // The exit is intentional even if diagnostics fail.
		os.Exit(3)
	case "where":
		return f.where(ctx, invocation)
	case "echo", "first":
		return f.echo(ctx, invocation)
	case "spin":
		if err := invocation.Output.Stdout(ctx, []byte("started")); err != nil {
			return result, err
		}
		// Deliberately refuses cancellation; the SDK must retire this child process.
		var spins atomic.Uint64
		for {
			spins.Add(1)
		}
	case "result":
		return f.result(ctx, invocation)
	default:
		return result, fmt.Errorf("unknown operation: %s", request.Operation)
	}
	return result, nil
}

func (f *Fixture) heldLocked() []string {
	held := make([]string, 0, len(f.conversations))
	for conversation := range f.conversations {
		held = append(held, conversation)
	}
	slices.Sort(held)
	return held
}

// Conversation answers retained-state queries and exercises controlled release failures.
func (f *Fixture) Conversation(
	ctx context.Context,
	conversation commandsdk.ConversationContext,
) (commandproto.Completion, error) {
	result := commandproto.Completion{}
	f.mu.Lock()
	f.initLocked()
	var data []byte
	var err error
	stall := false
	switch request := conversation.Request.(type) {
	case *commandproto.ConversationRelease:
		if f.stalling[request.Conversation] {
			select {
			case f.stalled <- struct{}{}:
			default:
			}
			f.mu.Unlock()
			<-ctx.Done()
			return result, ctx.Err()
		}
		if request.Conversation == "fail" {
			f.mu.Unlock()
			return result, errors.New("fixture cleanup failed")
		}
		delete(f.conversations, request.Conversation)
		data, err = (releaseAnswer{}).MarshalJSON()
	case *commandproto.ConversationQuery:
		if f.conversations["unanswerable"] {
			f.mu.Unlock()
			return result, errors.New("fixture status unavailable")
		}
		data, err = (commandproto.ConversationStatus{Conversations: f.heldLocked()}).MarshalJSON()
		stall = f.conversations["stall"]
	}
	stalled, proceed := f.stalled, f.proceed
	f.mu.Unlock()
	if err != nil {
		return result, err
	}
	if stall {
		select {
		case stalled <- struct{}{}:
		default:
		}
		select {
		case <-ctx.Done():
			return result, ctx.Err()
		case <-proceed:
		}
	}
	return result, conversation.Output.Stdout(ctx, data)
}

func (f *Fixture) drawNumbers(
	ctx context.Context,
	invocation commandsdk.InvocationContext[commandproto.Invocation],
	numbers *commandsdk.Numbers,
) (commandproto.Completion, error) {
	result := commandproto.Completion{}
	request := invocation.Request
	if numbers == nil {
		return result, errors.New("no numbers source")
	}
	args, err := decodeFixtureArgs(request.Args)
	if err != nil {
		return result, err
	}
	rawCount := string(args["count"])
	count, err := strconv.ParseUint(rawCount, 10, 64)
	if err != nil {
		count = 1
	}
	if count > math.MaxUint32 {
		return result, errors.New("out of range integral type conversion attempted")
	}
	first, err := numbers.Draw(ctx, request.Context.Conversation, commandproto.TabSequence, uint32(count))
	if err != nil {
		return result, err
	}
	data, err := (numberAnswer{First: first}).MarshalJSON()
	if err != nil {
		return result, err
	}
	return result, invocation.Output.Stdout(ctx, data)
}

func (f *Fixture) where(
	ctx context.Context,
	invocation commandsdk.InvocationContext[commandproto.Invocation],
) (commandproto.Completion, error) {
	result := commandproto.Completion{}
	request := invocation.Request
	args, err := decodeFixtureArgs(request.Args)
	if err != nil {
		return result, err
	}
	label := []byte("null")
	if value, ok := args["label"]; ok {
		label = value
	}
	var value *string
	if v, ok := request.Env["PROBE"]; ok {
		value = &v
	}
	data, err := (whereAnswer{Label: label, Context: request.Context, Cwd: request.Cwd, Value: value}).MarshalJSON()
	if err != nil {
		return result, err
	}
	return result, invocation.Output.Stdout(ctx, data)
}

func (f *Fixture) echo(
	ctx context.Context,
	invocation commandsdk.InvocationContext[commandproto.Invocation],
) (commandproto.Completion, error) {
	result := commandproto.Completion{}
	request := invocation.Request
	for {
		data, err := invocation.Input.Next(ctx)
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return result, err
		}
		if err := invocation.Output.Stdout(ctx, data); err != nil {
			return result, err
		}
		if request.Operation == "first" {
			break
		}
	}
	return result, nil
}

func (f *Fixture) result(
	ctx context.Context,
	invocation commandsdk.InvocationContext[commandproto.Invocation],
) (commandproto.Completion, error) {
	result := commandproto.Completion{}
	request := invocation.Request
	if err := invocation.Output.Stdout(ctx, []byte("command output")); err != nil {
		return result, err
	}
	if err := invocation.Output.Stderr(ctx, []byte("command diagnostic")); err != nil {
		return result, err
	}
	if request.Env["RESULT"] == "error" {
		return result, errors.New("command failed")
	}
	result.ExitCode = 17
	return result, nil
}

func (f *Fixture) heldAnswer(ctx context.Context, output *commandsdk.Output) (commandproto.Completion, error) {
	result := commandproto.Completion{}
	f.mu.Lock()
	held := f.heldLocked()
	f.mu.Unlock()
	data, err := (commandproto.ConversationStatus{Conversations: held}).MarshalJSON()
	if err != nil {
		return result, err
	}
	return result, output.Stdout(ctx, data)
}

func (f *Fixture) retain(request commandproto.Invocation) {
	f.mu.Lock()
	f.conversations[request.Context.Conversation] = true
	if request.Operation == "stall_release" {
		f.stalling[request.Context.Conversation] = true
	}
	f.mu.Unlock()
}
