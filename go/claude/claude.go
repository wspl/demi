// Package claude is the demi.claude command package: Demi's own verified copy
// of the Claude Code CLI on the machine that runs it
// (docs/providers/claude-code.md § The package). Its handler serves
// claude.ensure, which installs the version a release record names, and
// claude.status, which lists the installations; each answers one JSON document
// on standard output, a failure included ([claudeproto]).
package claude

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"

	"github.com/wspl/demi/go/claudeproto"
	"github.com/wspl/demi/go/commandservice"
)

// maxInputBytes is the most bytes a release record may have: it is a few hundred
// bytes for each platform.
const maxInputBytes = 64 * 1024

// A Handler serves the package's operations over a command service. The service
// holds no conversation state, so the SDK's empty conversation status and its
// no-op close stand.
type Handler struct {
	installer *Installer
}

// New returns a handler that installs into the default roots of this machine.
func New() *Handler {
	return NewHandler(NewInstaller(DefaultRoots()))
}

// NewHandler returns a handler that installs with installer.
func NewHandler(installer *Installer) *Handler {
	return &Handler{installer: installer}
}

// Operations returns the operations the handler serves.
func (h *Handler) Operations() []string {
	return claudeproto.Operations()
}

// Invoke runs one invocation: it reads the release record from the input for
// claude.ensure, and writes one document line. A failure has its code, and is
// said in the completion too; a cancelled invocation writes no document.
func (h *Handler) Invoke(call *commandservice.Call) (commandservice.Completion, error) {
	ctx := call.Context()
	var reply claudeproto.Reply
	var err error
	switch call.Invocation.Operation {
	case claudeproto.OperationEnsure:
		reply, err = h.ensure(ctx, call.Stdin)
	case claudeproto.OperationStatus:
		reply, err = h.status()
	default:
		return commandservice.Completion{}, fmt.Errorf("unknown operation %s", call.Invocation.Operation)
	}
	completion := commandservice.Completion{}
	if err != nil {
		// Only a cancellation has no code; it writes no document.
		if isCancellation(err) {
			return commandservice.Completion{}, commandservice.ErrCancelled
		}
		var ensure *EnsureError
		errors.As(installFailed(err), &ensure)
		reply = claudeproto.Failed{Code: ensure.Code, Message: ensure.Error()}
		completion = commandservice.Completion{
			ExitCode: 1,
			Error:    &commandservice.CommandError{Code: string(ensure.Code), Message: ensure.Error()},
		}
	}
	document, err := claudeproto.Encode(reply)
	if err != nil {
		return commandservice.Completion{}, err
	}
	if _, err := call.Stdout.Write(append(document, '\n')); err != nil {
		return commandservice.Completion{}, err
	}
	return completion, nil
}

func (h *Handler) ensure(ctx context.Context, input *commandservice.Input) (claudeproto.Reply, error) {
	bytes, err := readInput(ctx, input)
	if err != nil {
		return nil, err
	}
	release, err := h.installer.Release(bytes)
	if err != nil {
		return nil, err
	}
	installed, err := h.installer.Ensure(ctx, release)
	if err != nil {
		return nil, err
	}
	return claudeproto.Ensured{OK: true, Version: installed.Version, Path: installed.Path}, nil
}

func (h *Handler) status() (claudeproto.Reply, error) {
	status, err := h.installer.Status()
	if err != nil {
		return nil, err
	}
	return claudeproto.Listed{OK: true, Platform: status.Platform, Installed: status.Installed}, nil
}

// readInput reads the invocation's input to its end, refusing more than
// maxInputBytes.
func readInput(ctx context.Context, input *commandservice.Input) ([]byte, error) {
	var bytes []byte
	for {
		chunk, err := input.Next()
		switch {
		case err == io.EOF:
			return bytes, nil
		case errors.Is(err, commandservice.ErrCancelled), ctx.Err() != nil:
			return nil, context.Canceled
		case err != nil:
			return nil, invalidRelease(err.Error())
		case len(bytes)+len(chunk) > maxInputBytes:
			return nil, invalidRelease(fmt.Sprintf("input exceeds %d bytes", maxInputBytes))
		}
		bytes = append(bytes, chunk...)
	}
}

// Main serves the package over standard input and output, as a runner starts it,
// and exits the process: with status 0 when the service ended, else it says why
// on standard error and exits with status 1. Any other invocation of the program
// than --command-service exits with status 2.
func Main() {
	if len(os.Args) < 2 || os.Args[1] != "--command-service" {
		fmt.Fprintln(os.Stderr, "Usage: demi-claude --command-service")
		os.Exit(2)
	}
	if err := commandservice.ServeStdio(context.Background(), New()); err != nil {
		fmt.Fprintf(os.Stderr, "demi-claude: %v\n", err)
		os.Exit(1)
	}
	os.Exit(0)
}
