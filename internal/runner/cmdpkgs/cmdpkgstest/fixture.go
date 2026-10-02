package cmdpkgstest

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

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/commandwire/commandwiretest"
)

// NoNumbers refuses requests as a backend does for a conversation not on the device.
type NoNumbers struct{}

// Reserve refuses every request with "this test gives out no conversation numbers".
func (NoNumbers) Reserve(context.Context, string, commandwire.ServiceSequence, uint32) (uint64, error) {
	return 0, errors.New("this test gives out no conversation numbers")
}

// Fixture supplies the deliberately faulty demi-native-fixture service operations.
// Its zero value is ready to pass to cmdsdk.ServeStdio. Crash and spin operations
// must only run in the fixture child process, never in the test's own process.
type Fixture struct {
	mu               sync.Mutex
	conversations    map[string]bool
	stalling         map[string]bool
	stalled, proceed chan struct{}
	numbers          *cmdsdk.Numbers
}

func (f *Fixture) init() {
	if f.conversations == nil {
		f.conversations = make(map[string]bool)
		f.stalling = make(map[string]bool)
		f.stalled = make(chan struct{}, 1)
		f.proceed = make(chan struct{}, 1)
	}
}

// Operations returns the shared commandwiretest fixture operation catalog.
func (f *Fixture) Operations() []string { return commandwiretest.FixtureOperations() }

// SetNumbers supplies this connection's conversation number source.
func (f *Fixture) SetNumbers(numbers *cmdsdk.Numbers) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.numbers = numbers
}

// Invoke runs a fixture operation with its supplied IO and cancellation.
func (f *Fixture) Invoke(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	f.mu.Lock()
	f.init()
	numbers, stalled, proceed := f.numbers, f.stalled, f.proceed
	f.mu.Unlock()
	result := commandwire.Completion{}
	request := invocation.Request
	switch request.Operation {
	case "number":
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
		first, err := numbers.Draw(ctx, request.Context.Conversation, commandwire.TabSequence, uint32(count))
		if err != nil {
			return result, err
		}
		data, err := (numberAnswer{First: first}).MarshalJSON()
		if err != nil {
			return result, err
		}
		return result, invocation.Output.Stdout(ctx, data)
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
		f.mu.Lock()
		f.conversations[request.Context.Conversation] = true
		if request.Operation == "stall_release" {
			f.stalling[request.Context.Conversation] = true
		}
		f.mu.Unlock()
	case "held":
		f.mu.Lock()
		held := f.held()
		f.mu.Unlock()
		data, err := (commandwire.ConversationStatus{Conversations: held}).MarshalJSON()
		if err != nil {
			return result, err
		}
		return result, invocation.Output.Stdout(ctx, data)
	case "crash":
		_, _ = fmt.Fprintln(os.Stderr, "fixture crashing on purpose") // The exit is intentional even if diagnostics fail.
		os.Exit(3)
	case "where":
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
	case "echo", "first":
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
	default:
		return result, fmt.Errorf("unknown operation: %s", request.Operation)
	}
	return result, nil
}
func (f *Fixture) held() []string {
	held := make([]string, 0, len(f.conversations))
	for conversation := range f.conversations {
		held = append(held, conversation)
	}
	slices.Sort(held)
	return held
}

// Conversation answers retained-state queries and exercises controlled release failures.
func (f *Fixture) Conversation(ctx context.Context, conversation cmdsdk.ConversationContext) (commandwire.Completion, error) {
	result := commandwire.Completion{}
	f.mu.Lock()
	f.init()
	var data []byte
	var err error
	stall := false
	switch request := conversation.Request.(type) {
	case *commandwire.ConversationRelease:
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
	case *commandwire.ConversationQuery:
		if f.conversations["unanswerable"] {
			f.mu.Unlock()
			return result, errors.New("fixture status unavailable")
		}
		data, err = (commandwire.ConversationStatus{Conversations: f.held()}).MarshalJSON()
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
