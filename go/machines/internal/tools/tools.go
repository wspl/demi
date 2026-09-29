// Package tools runs the external programs the machine manager keeps
// (docs/cloud/managed-hosts.md § Linux control): runsc, mke2fs, e2fsck,
// resize2fs, bsdtar and nft. Each starts with the C locale's UTF-8 variant so
// its messages have one form, and a cancelled context kills a running one.
package tools

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"slices"
	"strings"
	"time"
	"unicode/utf8"
)

// A Tool is a program the manager runs.
type Tool int

// The programs the manager runs.
const (
	Runsc Tool = iota
	Mke2fs
	E2fsck
	Resize2fs
	Bsdtar
	Nft
)

// onPath are the programs found on PATH; runsc is configured.
var onPath = []Tool{Mke2fs, E2fsck, Resize2fs, Bsdtar, Nft}

func (t Tool) String() string {
	switch t {
	case Runsc:
		return "runsc"
	case Mke2fs:
		return "mke2fs"
	case E2fsck:
		return "e2fsck"
	case Resize2fs:
		return "resize2fs"
	case Bsdtar:
		return "bsdtar"
	case Nft:
		return "nft"
	}
	return "unknown"
}

// Tools are the resolved programs.
type Tools struct {
	paths map[Tool]string
}

// MissingError means programs the manager needs are not installed.
type MissingError struct {
	Names []string
}

func (e *MissingError) Error() string {
	return "Cloud manager needs: " + strings.Join(e.Names, ", ")
}

// Resolve finds every program: runsc at its configured path, the others on PATH.
func Resolve(runsc string) (*Tools, error) {
	tools := &Tools{paths: map[Tool]string{Runsc: runsc}}
	var missing []string
	for _, tool := range onPath {
		path, err := exec.LookPath(tool.String())
		if err != nil {
			missing = append(missing, tool.String())
			continue
		}
		tools.paths[tool] = path
	}
	if info, err := os.Stat(runsc); err != nil || !info.Mode().IsRegular() {
		missing = append(missing, Runsc.String())
	}
	if len(missing) > 0 {
		return nil, &MissingError{Names: missing}
	}
	return tools, nil
}

// Path returns the program's path.
func (t *Tools) Path(tool Tool) string {
	return t.paths[tool]
}

// Command returns a command for tool with the manager's environment and no
// input; cancelling ctx kills it.
func (t *Tools) Command(ctx context.Context, tool Tool, args ...string) *exec.Cmd {
	command := exec.CommandContext(ctx, t.Path(tool), args...)
	command.Env = append(os.Environ(), "LC_ALL=C.UTF-8")
	// A program whose pipes outlive its kill must not hold its caller.
	command.WaitDelay = 5 * time.Second
	return command
}

// A Kind says how a program failed.
type Kind int

// The ways a program fails.
const (
	// Spawn means it could not start, or could not be waited for.
	Spawn Kind = iota
	// Deadline means it did not finish in time.
	Deadline
	// Failed means it exited with a status the caller does not accept.
	Failed
)

// An Error is a program that could not run or did not succeed. The message
// carries the end of its output, which names what went wrong.
type Error struct {
	Tool Tool
	Kind Kind
	// Err is why a program could not start.
	Err error
	// Limit is the deadline a program missed.
	Limit time.Duration
	// Output is what a failed program wrote.
	Output Output
}

func (e *Error) Error() string {
	switch e.Kind {
	case Deadline:
		return fmt.Sprintf("%s did not finish within %d s", e.Tool, int64(e.Limit/time.Second))
	case Failed:
		return fmt.Sprintf("%s exited %s: %s", e.Tool, e.Output.describe(), e.Output.Message())
	}
	return fmt.Sprintf("%s could not start: %v", e.Tool, e.Err)
}

func (e *Error) Unwrap() error { return e.Err }

// Output is what a program wrote, decoded lossily: tools write text.
type Output struct {
	// Code is the exit status, or -1 for a program a signal ended.
	Code   int
	Stdout string
	Stderr string
}

// Success reports whether the program exited 0.
func (o Output) Success() bool { return o.Code == 0 }

func (o Output) describe() string {
	if o.Code < 0 {
		return "by signal"
	}
	return fmt.Sprintf("%d", o.Code)
}

// Message returns the error output, or the standard output when there is none,
// trimmed and bounded to its last 8 KiB.
func (o Output) Message() string {
	text := strings.TrimSpace(o.Stderr)
	if text == "" {
		text = strings.TrimSpace(o.Stdout)
	}
	return Tail(text, 8*1024)
}

// Tail returns the last bytes bytes of text, cut at a character boundary.
func Tail(text string, bytes int) string {
	start := max(len(text)-bytes, 0)
	for start < len(text) && !utf8.RuneStart(text[start]) {
		start++
	}
	return text[start:]
}

// Output runs tool to its end, within deadline when it is positive, and returns
// its output whatever its exit status.
func (t *Tools) Output(ctx context.Context, tool Tool, args []string, deadline time.Duration) (Output, error) {
	if deadline > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, deadline)
		defer cancel()
	}
	command := t.Command(ctx, tool, args...)
	var stdout, stderr bytes.Buffer
	command.Stdout = &stdout
	command.Stderr = &stderr
	err := command.Run()
	if deadline > 0 && errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return Output{}, &Error{Tool: tool, Kind: Deadline, Limit: deadline}
	}
	var exit *exec.ExitError
	if err != nil && !errors.As(err, &exit) {
		return Output{}, &Error{Tool: tool, Kind: Spawn, Err: err}
	}
	return Output{
		Code:   command.ProcessState.ExitCode(),
		Stdout: strings.ToValidUTF8(stdout.String(), "�"),
		Stderr: strings.ToValidUTF8(stderr.String(), "�"),
	}, nil
}

// Run runs tool and requires it to exit 0.
func (t *Tools) Run(ctx context.Context, tool Tool, args []string, deadline time.Duration) (Output, error) {
	output, err := t.Output(ctx, tool, args, deadline)
	if err != nil {
		return Output{}, err
	}
	if !output.Success() {
		return Output{}, &Error{Tool: tool, Kind: Failed, Output: output}
	}
	return output, nil
}

// Accept requires output of tool to have exited with one of codes.
func Accept(tool Tool, output Output, codes ...int) (Output, error) {
	if slices.Contains(codes, output.Code) {
		return output, nil
	}
	return Output{}, &Error{Tool: tool, Kind: Failed, Output: output}
}
