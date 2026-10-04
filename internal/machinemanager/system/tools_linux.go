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

	"github.com/wspl/demi/internal/contract"
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
	default:
		return fmt.Sprintf("Tool(%d)", uint8(t))
	}
}

// String returns the program name.
func (t Tool) String() string {
	return t.Name()
}

// Tools holds resolved programs. A zero value supplies empty paths for tests
// that only build command lines. Use Resolve for production execution.
type Tools struct{ paths map[Tool]string }

// NewTools copies already resolved program paths. It supports fixtures that
// substitute infrastructure programs; production callers use Resolve.
func NewTools(paths map[Tool]string) *Tools {
	return &Tools{paths: maps.Clone(paths)}
}

// SpawnFailed reports that tool could not start or that waiting for it failed.
func SpawnFailed(tool Tool, err error) error {
	return fmt.Errorf("%s could not start: %w", tool, err)
}

// DeadlinePassed reports that tool did not finish within deadline.
func DeadlinePassed(tool Tool, deadline time.Duration) error {
	return fmt.Errorf("%s did not finish within %d s", tool, int64(deadline/time.Second))
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
	for _, tool := range []Tool{Mke2fs, E2fsck, Resize2fs, Bsdtar} {
		path, err := exec.LookPath(tool.Name())
		// Accept a program found through a relative PATH entry, made absolute
		// instead of carrying exec.ErrDot into Command.
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
		//nolint:staticcheck // User-visible text, kept byte for byte.
		return nil, fmt.Errorf("Cloud manager needs: %s", strings.Join(missing, ", "))
	}
	paths[Runsc] = runsc
	return NewTools(paths), nil
}

// Path returns the resolved path of tool.
func (t *Tools) Path(tool Tool) string {
	return t.paths[tool]
}

// Command prepares tool with LC_ALL=C.UTF-8 and no input. The caller owns
// Start and Wait, and must cancel and reap a started command on every path.
// Context cancellation kills the child; merely losing its reference does not.
func (t *Tools) Command(ctx context.Context, tool Tool) *exec.Cmd {
	command := exec.CommandContext(ctx, t.Path(tool))
	command.Env = append(os.Environ(), "LC_ALL=C.UTF-8")
	return command
}

// Output runs tool to completion and returns output regardless of exit status.
// A zero deadline means no timeout.
// Cancellation or timeout kills and reaps the child before returning.
func (t *Tools) Output(ctx context.Context, tool Tool, args []string, deadline time.Duration) (Output, error) {
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
			return Output{}, SpawnFailed(tool, err)
		}
		streams[i].reader = reader
		streams[i].writer = writer
	}
	command.Stdout = streams[0].writer
	command.Stderr = streams[1].writer
	if err := command.Start(); err != nil {
		return Output{}, SpawnFailed(tool, err)
	}
	var readers sync.WaitGroup
	for i := range streams {
		// Start succeeded: only the child may retain the writing ends now.
		_ = streams[i].writer.Close()
		readers.Go(func() {
			_, streams[i].err = io.Copy(&streams[i].data, streams[i].reader)
		})
	}
	defer watchToolCancellation(runCtx, streams[0].reader, streams[1].reader)()
	if deadline > 0 {
		defer startToolDeadline(deadline, cancel)()
	}
	err := command.Wait()
	readers.Wait()
	if readErr := errors.Join(streams[0].err, streams[1].err); readErr != nil {
		err = readErr
	}
	if err := toolOutputError(ctx, runCtx, tool, deadline, err); err != nil {
		return Output{}, err
	}
	return decodeToolOutput(tool, command.ProcessState, streams[0].data.Bytes(), streams[1].data.Bytes())
}

// Run runs tool and requires it to exit 0.
func (t *Tools) Run(ctx context.Context, tool Tool, args []string, deadline time.Duration) (Output, error) {
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
	status := "by signal"
	if output.Status.Exited() {
		status = strconv.Itoa(output.Status.ExitStatus())
	}
	return Output{}, fmt.Errorf("%s exited %s: %s", tool, status, output.Message())
}

var errToolDeadline = errors.New("infrastructure tool deadline")

func toolOutputError(ctx, runCtx context.Context, tool Tool, deadline time.Duration, err error) error {
	if err == nil {
		return nil
	}

	if errors.Is(context.Cause(runCtx), errToolDeadline) {
		return DeadlinePassed(tool, deadline)
	}
	if ctx.Err() != nil {
		return SpawnFailed(tool, ctx.Err())
	}
	var exit *exec.ExitError
	if !errors.As(err, &exit) {
		return SpawnFailed(tool, err)
	}
	return nil
}

// startToolDeadline returns cleanup that stops or joins the owned deadline callback.
func startToolDeadline(deadline time.Duration, cancel context.CancelCauseFunc) func() {
	// The deadline starts after spawning. Join an already-fired timer
	// callback before returning, as well as always reaping the child.
	fired := make(chan struct{})
	timer := time.AfterFunc(deadline, func() {
		defer close(fired)
		cancel(errToolDeadline)
	})
	return func() {
		if !timer.Stop() {
			<-fired
		}
	}
}

func decodeToolOutput(tool Tool, state *os.ProcessState, stdout, stderr []byte) (Output, error) {
	status, ok := state.Sys().(syscall.WaitStatus)
	if !ok {
		return Output{}, SpawnFailed(tool, errors.New("tool returned no Linux wait status"))
	}
	return Output{
		Status: status,
		Stdout: contract.LossyUTF8(stdout),
		Stderr: contract.LossyUTF8(stderr),
	}, nil
}

// watchToolCancellation returns cleanup that stops or joins the pipe interruption callback.
func watchToolCancellation(ctx context.Context, stdout, stderr *os.File) func() {
	interrupted := make(chan struct{})
	stopInterrupt := context.AfterFunc(ctx, func() {
		defer close(interrupted)
		for _, reader := range []*os.File{stdout, stderr} {
			_ = reader.Close()
		}
	})
	return func() {
		if !stopInterrupt() {
			<-interrupted
		}
	}
}
