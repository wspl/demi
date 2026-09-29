package shell

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"sync"
	"unicode/utf8"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandtree"
	"mvdan.cc/sh/v3/interp"
)

// Services leases a connection to the exact binding pinned by the manifest.
// The owner validates the manifest before Run, and retains the service while
// release has not been called. release runs on success, failure and cancellation.
type Services interface {
	Acquire(context.Context, commandtree.Binding) (*commandservice.Client, func(), error)
}

// RPCCall is the validated invocation sent to the backend. Argv includes root.
// The backend routes by its live job record, never by the environment.
type RPCCall struct {
	Invocation commandservice.Invocation
	Path       []string
	Argv       []string
	Live       bool
	Finite     bool
}

// RPC is the runner connection's callback bridge. It must stop on cancellation
// and return only after its output has drained.
type RPC interface {
	Invoke(context.Context, RPCCall, io.Reader, io.Writer, io.Writer) (uint8, error)
}

// Commands is a job's immutable, validated command manifest and live binding.
// Cancelling Lifetime revokes it. Hints, when supplied, returns a release
// function that clears the running hint on every outcome.
type Commands struct {
	Roots    map[string]commandtree.Node
	Context  commandservice.CommandContext
	Lifetime context.Context
	Services Services
	RPC      RPC
	Hints    func(context.Context, string) (func(), error)
}

// Command is a raw declared-command call, shared by the interpreter and G5c's
// authenticated local endpoint. Stdin remains caller-owned. Dispatch duplicates
// it for cancellable, demand-driven reads; nil means EOF.
type Command struct {
	Argv     []string
	Dir      string
	Env      map[string]string
	Stdin    *os.File
	Stdout   io.Writer
	Stderr   io.Writer
	Live     bool
	Recorder *commandservice.Recorder
}

// Dispatch parses and runs a declared command through its pinned service or
// callback. Usage and command failures are reported on Stderr with their exit
// code; an error means that reporting the failure itself failed.
func (commands *Commands) Dispatch(ctx context.Context, call Command) (uint8, error) {
	if call.Stdout == nil {
		call.Stdout = io.Discard
	}
	if call.Stderr == nil {
		call.Stderr = io.Discard
	}
	if len(call.Argv) == 0 {
		return 0, errors.New("missing command root")
	}
	root, ok := commands.Roots[call.Argv[0]]
	var err error
	if !ok {
		err = fmt.Errorf("%s: not a root command of this manifest", call.Argv[0])
	} else {
		err = commands.dispatch(ctx, root, call)
	}
	var status interp.ExitStatus
	switch {
	case err == nil:
		return 0, nil
	case errors.As(err, &status):
		return uint8(status), nil
	case errors.Is(err, context.Canceled), errors.Is(err, commandservice.ErrCancelled):
		return 130, nil
	case errors.Is(err, io.ErrClosedPipe), brokenPipe(err):
		return 141, nil
	default:
		if _, failure := fmt.Fprintf(call.Stderr, "demi-runner: %v\n", err); failure != nil {
			return 0, failure
		}
		return 1, nil
	}
}

func (s *scope) commands(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
	return func(ctx context.Context, args []string) error {
		commands := s.options.Commands
		if commands == nil {
			return next(ctx, args)
		}
		if _, ok := commands.Roots[args[0]]; !ok {
			return next(ctx, args)
		}
		hc := interp.HandlerCtx(ctx)
		input, ok := hc.Stdin.(*os.File)
		if !ok && hc.Stdin != nil {
			return errors.New("interpreter stdin is not a file")
		}
		env := environment(hc.Env)
		if hc.ClearEnv {
			env = map[string]string{}
		}
		code, err := commands.Dispatch(ctx, Command{Argv: args, Dir: hc.Dir, Env: env,
			Stdin: input, Stdout: hc.Stdout, Stderr: hc.Stderr, Live: s.options.Live && input == s.options.Stdin,
			Recorder: s.options.Recorder})
		if err != nil {
			return err
		}
		if code != 0 {
			return interp.ExitStatus(code)
		}
		return nil
	}
}

func (commands *Commands) dispatch(parent context.Context, root commandtree.Node, call Command) error {
	argv := call.Argv
	ctx, cancel := context.WithCancel(parent)
	defer cancel()
	if commands.Lifetime != nil {
		if commands.Lifetime.Err() != nil {
			return commandservice.ErrCancelled
		}
		stop := context.AfterFunc(commands.Lifetime, cancel)
		defer stop()
	}
	selected, err := commandtree.Select(root, argv[1:])
	if err != nil {
		return err
	}
	parsed, err := selected.Parse(argv[1:])
	if err != nil {
		return err
	}
	if parsed.Help {
		_, err := fmt.Fprintln(call.Stdout, commandtree.Help(selected.Node, strings.Join(selected.Path, " ")))
		return err
	}
	leaf, ok := selected.Node.(commandtree.Leaf)
	if !ok {
		return errors.New("missing command leaf")
	}
	inputFile := call.Stdin
	if inputFile == nil {
		file, err := openWaiting(ctx, os.DevNull)
		if err != nil {
			return err
		}
		defer file.Close()
		inputFile = file
	}
	inputCopy, err := commandservice.RetryBlocking(func() (*os.File, error) {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		return duplicateInput(inputFile)
	})
	if err != nil {
		return err
	}
	inputOwner, err := newInvocationInput(ctx, inputCopy)
	if err != nil {
		return err
	}
	stopInput := context.AfterFunc(ctx, inputOwner.close)
	defer func() {
		stopInput()
		inputOwner.close()
	}()
	var body *string
	if leaf.StdinField != nil {
		text := ""
		if !call.Live {
			data, err := io.ReadAll(io.LimitReader(inputOwner, 1024*1024+1))
			if err != nil {
				return err
			}
			if len(data) > 1024*1024 {
				return errors.New("command body exceeds 1 MiB")
			}
			if !utf8.Valid(data) {
				return errors.New("command body is not UTF-8")
			}
			text = string(data)
		}
		body = &text
	}
	parsed, err = parsed.Validate(leaf, body)
	if err != nil {
		return err
	}
	if leaf.RunningHint != nil && commands.Hints != nil {
		release, err := commands.Hints(ctx, *leaf.RunningHint)
		if err != nil {
			return err
		}
		defer release()
	}
	arguments, err := json.Marshal(parsed.Values)
	if err != nil {
		return err
	}
	request := commandservice.Invocation{InvocationID: rand.Text(), Context: commands.Context,
		Args: arguments, Cwd: call.Dir, Env: call.Env, JSON: &parsed.JSON}
	if call.Recorder != nil {
		edits := call.Recorder.Context()
		request.Edits = &edits
	}
	var input io.Reader = inputOwner
	if body != nil {
		input = strings.NewReader("")
	}
	output := call.Stdout
	var capture limitedOutput
	if parsed.JSON {
		output = &capture
	}
	var code uint8
	if leaf.Binding != nil {
		if commands.Services == nil {
			return errors.New("native service resolver is unavailable")
		}
		request.Operation = leaf.Binding.Operation
		client, release, err := commands.Services.Acquire(ctx, *leaf.Binding)
		if err != nil {
			return err
		}
		defer release()
		stream, err := client.Invoke(ctx, request)
		if err != nil {
			return err
		}
		defer stream.Cancel()
		completion, err := commandservice.Exchange(ctx, stream, input, output, call.Stderr)
		if err != nil {
			return err
		}
		code = completion.ExitCode
		if completion.Error != nil {
			if _, err := fmt.Fprintf(call.Stderr, "%s: %s\n", completion.Error.Code, completion.Error.Message); err != nil {
				return err
			}
		}
	} else {
		if commands.RPC == nil {
			return errors.New("command callback is unavailable")
		}
		code, err = commands.RPC.Invoke(ctx, RPCCall{Invocation: request, Path: parsed.Path,
			Argv: argv, Live: call.Live, Finite: !call.Live && body == nil}, input, output, call.Stderr)
		if err != nil {
			return err
		}
	}
	if code != 0 {
		return interp.ExitStatus(code)
	}
	if parsed.JSON {
		value, err := commandtree.DecodeValue(capture.Bytes())
		if err != nil {
			return fmt.Errorf("--json output is not JSON: %w", err)
		}
		if err := leaf.JSONOutput().Check(value); err != nil {
			return fmt.Errorf("--json output does not match its schema: %w", err)
		}
		_, err = call.Stdout.Write(capture.Bytes())
		return err
	}
	return nil
}

// limitedOutput holds only the bounded JSON result that can still be released.
type limitedOutput struct{ bytes.Buffer }

func (b *limitedOutput) Write(p []byte) (int, error) {
	if len(p) > 1024*1024-b.Len() {
		return 0, errors.New("--json output exceeds 1 MiB")
	}
	return b.Buffer.Write(p)
}

// invocationInput closes a call's copy of stdin on every outcome. Reads that
// the service SDK started end when the call does and return before dispatch
// does: the copy may be blocking (an external command the shell ran switched
// the shared pipe to blocking mode), so a read waits through inputWake, which
// the end of the call wakes, rather than in a read that closing cannot end.
type invocationInput struct {
	file    *os.File
	wake    *inputWake
	mu      sync.Mutex
	closed  bool
	reading sync.WaitGroup
	once    sync.Once
}

// newInvocationInput owns file, the call's copy of stdin; its wake waits out a
// lack of descriptors until ctx ends, as the copy did.
func newInvocationInput(ctx context.Context, file *os.File) (*invocationInput, error) {
	wake, err := commandservice.RetryBlocking(func() (*inputWake, error) {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		return newInputWake()
	})
	if err != nil {
		// Closing the unused copy cannot change the command's outcome.
		_ = file.Close()
		return nil, err
	}
	return &invocationInput{file: file, wake: wake}, nil
}

func (r *invocationInput) Read(p []byte) (int, error) {
	r.mu.Lock()
	if r.closed {
		r.mu.Unlock()
		return 0, os.ErrClosed
	}
	r.reading.Add(1)
	r.mu.Unlock()
	defer r.reading.Done()
	return r.wake.read(r.file, p)
}

// close ends the reads, waits for them to return and releases the copy; a
// second call waits for the first.
func (r *invocationInput) close() {
	r.once.Do(func() {
		r.mu.Lock()
		r.closed = true
		r.mu.Unlock()
		r.wake.interrupt(r.file)
		r.reading.Wait()
		r.wake.release(r.file)
	})
}
