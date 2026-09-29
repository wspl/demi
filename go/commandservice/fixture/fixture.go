// Package fixture is the runner's native fixture service: deliberately faulty
// operations for integration tests, and a conversation endpoint whose state the
// tests steer. Its Rust counterpart is crates/runner/tests/fixtures/service.rs,
// and the two answer alike, to the byte.
package fixture

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"maps"
	"math"
	"os"
	"slices"
	"sync"
	"sync/atomic"

	"github.com/wspl/demi/go/commandservice"
)

// Operations returns the operations of the fixture service, which the runner's
// and host-remote's tests name in their descriptors.
func Operations() []string {
	return []string{
		"where", "echo", "first", "spin", "result", "retain", "stall_release", "held", "crash", "stalled",
		"proceed", "number",
	}
}

// Main serves a fixture over standard input and output, and exits the process:
// with status 0 when the service ended normally, else it says why on standard
// error and exits with status 1.
func Main() {
	if err := commandservice.ServeStdio(context.Background(), New()); err != nil {
		fmt.Fprintf(os.Stderr, "fixture: %v\n", err)
		os.Exit(1)
	}
	os.Exit(0)
}

// A Service is the fixture service:
//
//	where          prints its label, context, working directory and $PROBE as JSON
//	echo, first    echo their input, all of it or the first chunk
//	spin           prints "started" and never returns
//	result         prints to both outputs and exits 17, or fails when $RESULT is error
//	retain         holds the invoking conversation
//	stall_release  holds it, and its release never ends by itself
//	held           prints the conversations held, as its status answers
//	crash          exits the whole process with status 3
//	stalled        ends once a status or a release waits to answer
//	proceed        lets that status answer
//	number         draws tab numbers of the invoking conversation
//
// A status waits for "proceed" while the fixture holds the conversation
// "stall", fails while it holds "unanswerable", and a release of "fail" fails.
type Service struct {
	mu sync.Mutex
	// conversations are the ones the service holds state for.
	conversations map[string]struct{}
	// stalling are the ones whose release never ends by itself.
	stalling map[string]struct{}

	// stalled holds a permit when a status taken while "stall" is held, or a
	// release of a stalling conversation, waits to answer. proceed holds one
	// to let that status answer with what it held when it was asked.
	stalled chan struct{}
	proceed chan struct{}
}

var (
	_ commandservice.Handler             = (*Service)(nil)
	_ commandservice.ConversationHandler = (*Service)(nil)
)

// New returns a fixture that holds no conversation.
func New() *Service {
	return &Service{
		conversations: map[string]struct{}{},
		stalling:      map[string]struct{}{},
		stalled:       make(chan struct{}, 1),
		proceed:       make(chan struct{}, 1),
	}
}

// Operations returns [Operations].
func (f *Service) Operations() []string { return Operations() }

// Invoke runs one of the fixture's operations.
func (f *Service) Invoke(call *commandservice.Call) (commandservice.Completion, error) {
	invocation := call.Invocation
	switch invocation.Operation {
	case "number":
		return commandservice.Completion{}, f.number(call)
	case "stalled":
		// It ignores the call's cancellation, as a Notify wait does.
		<-f.stalled
	case "proceed":
		notify(f.proceed)
	case "retain":
		f.hold(invocation.Context.Conversation, false)
	case "stall_release":
		f.hold(invocation.Context.Conversation, true)
	case "held":
		return commandservice.Completion{}, f.held(call)
	case "crash":
		fmt.Fprintln(os.Stderr, "fixture crashing on purpose")
		os.Exit(3)
	case "where":
		return commandservice.Completion{}, where(call)
	case "echo", "first":
		return commandservice.Completion{}, echo(call, invocation.Operation == "first")
	case "spin":
		return commandservice.Completion{}, spin(call)
	case "result":
		return result(call)
	default:
		return commandservice.Completion{}, fmt.Errorf("unknown operation %s", invocation.Operation)
	}
	return commandservice.Completion{}, nil
}

// notify leaves a permit in ch unless it holds one already.
func notify(ch chan struct{}) {
	select {
	case ch <- struct{}{}:
	default:
	}
}

func (f *Service) hold(conversation string, stalling bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.conversations[conversation] = struct{}{}
	if stalling {
		f.stalling[conversation] = struct{}{}
	}
}

// A statusReport is what the fixture's status prints, and what `held` prints.
type statusReport struct {
	Conversations []string `json:"conversations"`
}

func (f *Service) held(call *commandservice.Call) error {
	f.mu.Lock()
	report := statusReport{Conversations: slices.Sorted(maps.Keys(f.conversations))}
	f.mu.Unlock()
	return writeJSON(call.Stdout, report)
}

// number draws count tab numbers of the invoking conversation, one by default,
// and prints the first.
func (f *Service) number(call *commandservice.Call) error {
	args, err := argsOf(call)
	if err != nil {
		return err
	}
	count := uint64(1)
	if raw, ok := args["count"]; ok {
		var given uint64
		// Anything but a nonnegative integer leaves the default.
		if json.Unmarshal(raw, &given) == nil {
			count = given
		}
	}
	if count > math.MaxUint32 {
		return errors.New("out of range integral type conversion attempted")
	}
	first, err := call.Numbers.Draw(call.Context(), call.Invocation.Context.Conversation, commandservice.SequenceTab, int(count))
	if err != nil {
		return err
	}
	return writeJSON(call.Stdout, struct {
		First uint64 `json:"first"`
	}{first})
}

// A whereReport is what `where` prints, its members in the order the Rust
// fixture writes them.
type whereReport struct {
	Label   jsontext.Value                `json:"label"`
	Context commandservice.CommandContext `json:"context"`
	Cwd     string                        `json:"cwd"`
	Value   *string                       `json:"value"`
}

// where prints what the invocation carries: its label, context, working
// directory and $PROBE.
func where(call *commandservice.Call) error {
	args, err := argsOf(call)
	if err != nil {
		return err
	}
	report := whereReport{
		Label:   args["label"],
		Context: call.Invocation.Context,
		Cwd:     call.Invocation.Cwd,
	}
	if value, ok := call.Invocation.Env["PROBE"]; ok {
		report.Value = &value
	}
	return writeJSON(call.Stdout, report)
}

func argsOf(call *commandservice.Call) (map[string]jsontext.Value, error) {
	var args map[string]jsontext.Value
	if err := json.Unmarshal(call.Invocation.Args, &args); err != nil {
		return nil, err
	}
	return args, nil
}

func writeJSON(w io.Writer, value any) error {
	document, err := json.Marshal(value)
	if err != nil {
		return err
	}
	_, err = w.Write(document)
	return err
}

// echo writes its input back, all of it or only the first chunk.
func echo(call *commandservice.Call, firstOnly bool) error {
	for {
		chunk, err := call.Stdin.Next()
		if err == io.EOF {
			return nil
		}
		if err != nil {
			return err
		}
		if _, err := call.Stdout.Write(chunk); err != nil {
			return err
		}
		if firstOnly {
			return nil
		}
	}
}

// spin never returns, and ignores its cancellation, as a handler that does
// CPU-bound work without ever looking at its context does.
func spin(call *commandservice.Call) error {
	if _, err := io.WriteString(call.Stdout, "started"); err != nil {
		return err
	}
	var spins atomic.Uint64
	for {
		spins.Add(1)
	}
}

// result writes to both outputs and exits 17, or fails when $RESULT is error.
func result(call *commandservice.Call) (commandservice.Completion, error) {
	if _, err := io.WriteString(call.Stdout, "command output"); err != nil {
		return commandservice.Completion{}, err
	}
	if _, err := io.WriteString(call.Stderr, "command diagnostic"); err != nil {
		return commandservice.Completion{}, err
	}
	if call.Invocation.Env["RESULT"] == "error" {
		return commandservice.Completion{}, errors.New("command failed")
	}
	return commandservice.Completion{ExitCode: 17}, nil
}

// Conversation answers the conversation endpoint from what the fixture holds.
func (f *Service) Conversation(call *commandservice.ConversationCall) (commandservice.Completion, error) {
	request := call.Request
	if request.Operation == commandservice.OperationRelease && f.isStalling(request.Conversation) {
		notify(f.stalled)
		<-call.Context().Done()
		return commandservice.Completion{}, commandservice.ErrCancelled
	}
	answer, stall, err := f.answer(request)
	if err != nil {
		return commandservice.Completion{}, err
	}
	// While it holds "stall", the answer, which was read at once, arrives only
	// after "proceed", as a late one does; that wait ignores the call's
	// cancellation, as a Notify wait does.
	if stall {
		notify(f.stalled)
		<-f.proceed
	}
	return commandservice.Completion{}, writeJSON(call.Stdout, answer)
}

func (f *Service) isStalling(conversation string) bool {
	f.mu.Lock()
	defer f.mu.Unlock()
	_, stalling := f.stalling[conversation]
	return stalling
}

// answer takes what the request asks of the fixture's state, at once.
func (f *Service) answer(request commandservice.ConversationRequest) (answer any, stall bool, err error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	_, unanswerable := f.conversations["unanswerable"]
	_, stalls := f.conversations["stall"]
	switch {
	case request.Operation == commandservice.OperationStatus && unanswerable:
		// A service that cannot say what it holds.
		return nil, false, errors.New("fixture status unavailable")
	case request.Operation == commandservice.OperationStatus:
		return statusReport{Conversations: slices.Sorted(maps.Keys(f.conversations))}, stalls, nil
	case request.Conversation == "fail":
		return nil, false, errors.New("fixture cleanup failed")
	}
	delete(f.conversations, request.Conversation)
	return struct{}{}, false, nil
}
