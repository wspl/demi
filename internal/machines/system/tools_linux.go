//go:build linux

package system

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"maps"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
	"unicode/utf8"
)

// Tool identifies a program the manager runs.
type Tool uint8

const (
	// Runsc is the configured sandbox runtime.
	Runsc Tool = iota
	// Mke2fs creates ext4 images.
	Mke2fs
	// E2fsck checks and repairs ext4 images.
	E2fsck
	// Resize2fs grows ext4 images.
	Resize2fs
	// Bsdtar extracts base archives.
	Bsdtar
	// Nft applies firewall transactions.
	Nft
)

// Name returns the program name.
func (t Tool) Name() string {
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
	default:
		return fmt.Sprintf("Tool(%d)", uint8(t))
	}
}

// String returns the program name.
func (t Tool) String() string { return t.Name() }

// Tools holds resolved programs. A zero value supplies empty paths for tests
// that only build command lines. Use Resolve for production execution.
type Tools struct{ paths map[Tool]string }

// NewTools copies already resolved program paths. It supports fixtures that
// substitute infrastructure programs; production callers use Resolve.
func NewTools(paths map[Tool]string) *Tools { return &Tools{paths: maps.Clone(paths)} }

// MissingTools names programs that are not installed.
type MissingTools struct {
	// Names lists missing programs in lookup order.
	Names []string
}

// Error describes the missing programs.
func (e *MissingTools) Error() string { return "Cloud manager needs: " + strings.Join(e.Names, ", ") }

// SpawnError reports a program that could not start or whose wait failed.
type SpawnError struct {
	// Tool identifies the program.
	Tool Tool
	// Source is the start or wait failure.
	Source error
}

// Error describes the failure to run the tool.
func (e *SpawnError) Error() string { return fmt.Sprintf("%s could not start: %v", e.Tool, e.Source) }

// Unwrap preserves the underlying IO error.
func (e *SpawnError) Unwrap() error { return e.Source }

// DeadlineError reports a tool that did not finish within its deadline.
type DeadlineError struct {
	// Tool identifies the program.
	Tool Tool
	// Deadline is the permitted runtime.
	Deadline time.Duration
}

// Error describes the tool's exceeded deadline.
func (e *DeadlineError) Error() string {
	return fmt.Sprintf("%s did not finish within %d s", e.Tool, int64(e.Deadline/time.Second))
}

// FailedError reports a program whose exit status was not accepted.
type FailedError struct {
	// Tool identifies the program.
	Tool Tool
	// Output carries the rejected status and decoded output.
	Output Output
}

// Error describes the exit status and the kept tail of output.
func (e *FailedError) Error() string {
	status := "by signal"
	if e.Output.Status.Exited() {
		status = strconv.Itoa(e.Output.Status.ExitStatus())
	}
	return fmt.Sprintf("%s exited %s: %s", e.Tool, status, e.Output.Message())
}

// Output is what a program wrote, decoded lossily: tools write text.
type Output struct {
	// Status preserves both exit codes and termination by signal.
	Status syscall.WaitStatus
	// Stdout is standard output decoded with replacement for invalid UTF-8.
	Stdout string
	// Stderr is standard error decoded with replacement for invalid UTF-8.
	Stderr string
}

// Message returns stderr, or stdout when stderr is blank, trimmed and bounded
// to its last 8 KiB at a UTF-8 character boundary.
func (o Output) Message() string {
	text := strings.TrimSpace(o.Stderr)
	if text == "" {
		text = strings.TrimSpace(o.Stdout)
	}
	return Tail(text, 8*1024)
}

// Tail returns the last bytes of text, cut at a UTF-8 character boundary.
func Tail(text string, bytes int) string {
	if bytes <= 0 {
		return ""
	}
	start := max(0, len(text)-bytes)
	for start < len(text) && !utf8.RuneStart(text[start]) {
		start++
	}
	return text[start:]
}

// Resolve finds runsc at its configured path and the other programs on PATH.
func Resolve(ctx context.Context, runsc string) (*Tools, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	paths := make(map[Tool]string)
	var missing []string
	for _, tool := range []Tool{Mke2fs, E2fsck, Resize2fs, Bsdtar, Nft} {
		path, err := exec.LookPath(tool.Name())
		// Rust's which accepts a program found through a relative PATH entry.
		// Make that explicit instead of carrying exec.ErrDot into Command.
		if errors.Is(err, exec.ErrDot) {
			path, err = filepath.Abs(path)
		}
		if err != nil {
			missing = append(missing, tool.Name())
		} else {
			paths[tool] = path
		}
	}
	info, err := os.Stat(runsc)
	if err != nil || !info.Mode().IsRegular() {
		missing = append(missing, Runsc.Name())
	}
	if len(missing) > 0 {
		return nil, &MissingTools{Names: missing}
	}
	paths[Runsc] = runsc
	return NewTools(paths), nil
}

// Path returns the resolved path of tool.
func (t *Tools) Path(tool Tool) string { return t.paths[tool] }

// Command prepares tool with LC_ALL=C.UTF-8 and no input. The caller owns
// Start and Wait, and must cancel and reap a started command on every path.
// Context cancellation kills the child; merely losing its reference does not.
func (t *Tools) Command(ctx context.Context, tool Tool) *exec.Cmd {
	command := exec.CommandContext(ctx, t.Path(tool))
	command.Env = append(os.Environ(), "LC_ALL=C.UTF-8")
	return command
}

// Output runs tool to completion and returns output regardless of exit status.
// A nil deadline means no timeout; a non-nil zero duration expires immediately.
// Cancellation or timeout kills and reaps the child before returning.
func (t *Tools) Output(ctx context.Context, tool Tool, args []string, deadline *time.Duration) (Output, error) {
	runCtx, cancel := context.WithCancelCause(ctx)
	defer cancel(nil)
	command := t.Command(runCtx, tool)
	command.Args = append(command.Args, args...)
	// Own the output pipes ourselves. exec.Cmd's implicit copy goroutines can
	// otherwise wait forever when a descendant retains an output descriptor.
	var streams [2]struct {
		reader, writer *os.File
		data           bytes.Buffer
		err            error
	}
	defer func() {
		// Pipes hold no buffered file writes, and are closed on every path.
		for i := range streams {
			if streams[i].reader != nil {
				_ = streams[i].reader.Close()
			}
			if streams[i].writer != nil {
				_ = streams[i].writer.Close()
			}
		}
	}()
	for i := range streams {
		reader, writer, err := os.Pipe()
		if err != nil {
			return Output{}, &SpawnError{Tool: tool, Source: err}
		}
		streams[i].reader = reader
		streams[i].writer = writer
	}
	command.Stdout = streams[0].writer
	command.Stderr = streams[1].writer
	if err := command.Start(); err != nil {
		return Output{}, &SpawnError{Tool: tool, Source: err}
	}
	var readers sync.WaitGroup
	for i := range streams {
		// Start succeeded: only the child may retain the writing ends now.
		_ = streams[i].writer.Close()
		readers.Go(func() { _, streams[i].err = io.Copy(&streams[i].data, streams[i].reader) })
	}
	interrupted := make(chan struct{})
	stopInterrupt := context.AfterFunc(runCtx, func() {
		defer close(interrupted)
		for i := range streams {
			_ = streams[i].reader.Close()
		}
	})
	defer func() {
		if !stopInterrupt() {
			<-interrupted
		}
	}()
	if deadline != nil {
		// Start the deadline after spawning, as Rust does. Join an already-fired
		// timer callback before returning, as well as always reaping the child.
		fired := make(chan struct{})
		timer := time.AfterFunc(*deadline, func() {
			defer close(fired)
			cancel(errToolDeadline)
		})
		defer func() {
			if !timer.Stop() {
				<-fired
			}
		}()
	}
	err := command.Wait()
	readers.Wait()
	if readErr := errors.Join(streams[0].err, streams[1].err); readErr != nil {
		err = readErr
	}
	if err != nil {
		if errors.Is(context.Cause(runCtx), errToolDeadline) {
			return Output{}, &DeadlineError{Tool: tool, Deadline: *deadline}
		}
		if ctx.Err() != nil {
			return Output{}, &SpawnError{Tool: tool, Source: ctx.Err()}
		}
		var exit *exec.ExitError
		if !errors.As(err, &exit) {
			return Output{}, &SpawnError{Tool: tool, Source: err}
		}
	}
	status, ok := command.ProcessState.Sys().(syscall.WaitStatus)
	if !ok {
		return Output{}, &SpawnError{Tool: tool, Source: errors.New("tool returned no Linux wait status")}
	}
	return Output{Status: status, Stdout: toolText(streams[0].data.Bytes()), Stderr: toolText(streams[1].data.Bytes())}, nil
}

// Run runs tool and requires it to exit 0.
func (t *Tools) Run(ctx context.Context, tool Tool, args []string, deadline *time.Duration) (Output, error) {
	output, err := t.Output(ctx, tool, args, deadline)
	if err != nil {
		return Output{}, err
	}
	return Accept(tool, output, []int{0})
}

// Accept requires output of tool to have exited with one of codes.
func Accept(tool Tool, output Output, codes []int) (Output, error) {
	if output.Status.Exited() && slices.Contains(codes, output.Status.ExitStatus()) {
		return output, nil
	}
	return Output{}, &FailedError{Tool: tool, Output: output}
}

var errToolDeadline = errors.New("infrastructure tool deadline")

// toolText decodes infrastructure output with Rust's replacement for each
// malformed UTF-8 subsequence. strings.ToValidUTF8 merges adjacent malformed
// subsequences; rune conversion instead splits a truncated valid prefix.
func toolText(data []byte) string {
	var text strings.Builder
	for len(data) > 0 {
		r, size := utf8.DecodeRune(data)
		if r != utf8.RuneError || size != 1 {
			text.Write(data[:size])
			data = data[size:]
			continue
		}
		size = 1
		first := data[0]
		expected := 0
		switch {
		case first >= 0xc2 && first <= 0xdf:
			expected = 2
		case first >= 0xe0 && first <= 0xef:
			expected = 3
		case first >= 0xf0 && first <= 0xf4:
			expected = 4
		}
		for size < expected && size < len(data) {
			next := data[size]
			if next < 0x80 || next > 0xbf {
				break
			}
			if size == 1 && (first == 0xe0 && next < 0xa0 || first == 0xed && next >= 0xa0 || first == 0xf0 && next < 0x90 || first == 0xf4 && next >= 0x90) {
				break
			}
			size++
		}
		text.WriteRune(utf8.RuneError)
		data = data[size:]
	}
	return text.String()
}
