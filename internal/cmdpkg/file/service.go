//nolint:staticcheck // Command error messages are copied verbatim from the Rust service.
package file

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/cmdpkg/file/fileop"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/gates"
)

// Serve validates the launch argument and serves the resident file service.
// The executable must exit when Serve returns.
func Serve(ctx context.Context, args []string) error {
	if _, err := cmdsdk.ParseLaunch(args); err != nil {
		return err
	}
	return cmdsdk.ServeStdio(ctx, &service{})
}

type service struct{ mutations gates.Serial }

func (*service) Operations() []string { return fileop.Operations() }
func failure(err error) commandwire.Completion {
	return commandwire.Completion{ExitCode: 1, Error: &commandwire.CommandError{Code: "command_failed", Message: err.Error()}}
}
func (s *service) Invoke(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	op, err := fileop.Parse(invocation.Request.Operation, invocation.Request.Args)
	if err != nil {
		var opErr *fileop.OperationError
		if errors.As(err, &opErr) && opErr.Err == nil {
			return commandwire.Completion{}, err
		}
		return failure(err), nil
	}
	if args, ok := op.(*fileop.ReadArgs); ok {
		return read(ctx, invocation, args.Path)
	}
	permit, err := s.mutations.Acquire(ctx)
	if err != nil {
		return commandwire.Completion{}, err
	}
	defer permit.Release()
	message, err := mutate(ctx, invocation.Request, op)
	if err != nil {
		if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return commandwire.Completion{}, err
		}
		return failure(err), nil
	}
	return commandwire.Completion{}, invocation.Output.Stdout(ctx, []byte(message))
}

func read(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.Invocation], name string) (commandwire.Completion, error) {
	path, err := cmdsdk.Resolve(invocation.Request.Cwd, name)
	if err != nil {
		return failure(err), nil
	}
	f, err := os.Open(path)
	if err != nil {
		return failure(err), nil
	}
	closed := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		// Closing the read handle interrupts a pending cancellable read.
		_ = f.Close()
		close(closed)
	})
	defer func() {
		if !stop() {
			<-closed
		}
		// A read-only close cannot affect bytes already delivered.
		_ = f.Close()
	}()
	buffer := make([]byte, 64*1024)
	for {
		if err := ctx.Err(); err != nil {
			return commandwire.Completion{}, err
		}
		n, err := f.Read(buffer)
		if cancelErr := ctx.Err(); cancelErr != nil {
			return commandwire.Completion{}, cancelErr
		}
		if n > 0 {
			if outputErr := invocation.Output.Stdout(ctx, buffer[:n]); outputErr != nil {
				return commandwire.Completion{}, outputErr
			}
		}
		if errors.Is(err, io.EOF) {
			return commandwire.Completion{}, nil
		}
		if err != nil {
			return failure(err), nil
		}
	}
}

// mutate records file publication and rollback while the caller holds the mutation gate.
func mutate(ctx context.Context, request commandwire.Invocation, op fileop.Operation) (string, error) {
	var recording *cmdsdk.Recording
	if request.Edits != nil {
		recorder, err := cmdsdk.NewRecorder(ctx, *request.Edits)
		if err != nil {
			slog.Warn("edit recording failed", "error", err)
		} else {
			recording = recorder.Begin(ctx)
		}
	}
	if recording != nil {
		defer recording.Close(ctx)
	}
	if err := ctx.Err(); err != nil {
		return "", err
	}
	switch args := op.(type) {
	case *fileop.CreateArgs:
		path, err := cmdsdk.Resolve(request.Cwd, args.Path)
		if err != nil {
			return "", err
		}
		if recording != nil {
			recording.Track(ctx, path)
		}
		if err := ctx.Err(); err != nil {
			return "", err
		}
		if err := atomicWrite(ctx, path, []byte(args.Content), true); err != nil {
			return "", err
		}
		return fmt.Sprintf("Created %s\n", args.Path), nil
	case *fileop.EditArgs:
		return edit(ctx, request.Cwd, args, recording)
	case *fileop.PatchArgs:
		return applyPatch(ctx, request.Cwd, args.Patch, recording)
	case *fileop.ReadArgs:
		return "", errors.New("read is not a mutation")
	}
	return "", errors.New("unknown file operation")
}

// atomicWrite publishes file contents beside their destination, preserving edit symlinks.
func atomicWrite(ctx context.Context, path string, data []byte, create bool) error {
	if !create {
		if info, err := os.Lstat(path); err == nil && info.Mode()&os.ModeSymlink != 0 {
			resolved, err := filepath.EvalSymlinks(path)
			if err != nil {
				return err
			}
			path = resolved
		}
	}
	parent := filepath.Dir(path)
	if parent == path {
		return errors.New("File has no parent directory")
	}
	if err := os.MkdirAll(parent, 0777); err != nil {
		return err
	}
	publication := artifacts.Publication{Mode: artifacts.Replace, Permissions: artifacts.Keep, Durable: true}
	if create {
		publication.Mode = artifacts.CreateNew
		publication.Permissions = artifacts.Default
	}
	return artifacts.PublishBytes(context.WithoutCancel(ctx), path, data, publication)
}

func edit(ctx context.Context, cwd string, args *fileop.EditArgs, recording *cmdsdk.Recording) (string, error) {
	path, err := cmdsdk.Resolve(cwd, args.Path)
	if err != nil {
		return "", err
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	if !utf8.Valid(data) {
		return "", errors.New("stream did not contain valid UTF-8")
	}
	content := string(data)
	var matches []int
	for offset := 0; offset <= len(content); {
		index := strings.Index(content[offset:], args.Old)
		if index < 0 {
			break
		}
		index += offset
		matches = append(matches, index)
		offset = index + len(args.Old)
	}
	if err := ctx.Err(); err != nil {
		return "", err
	}
	index := 0
	switch {
	case args.Occurrence != nil:
		if *args.Occurrence > uint(len(matches)) {
			return "", fmt.Errorf("Occurrence %d is out of range", *args.Occurrence)
		}
		index = matches[*args.Occurrence-1]
	case args.Context != nil:
		if len(matches) == 0 {
			return "", errors.New("No match found")
		}
		distance := ^uint(0)
		ambiguous := false
		for _, candidate := range matches {
			line := uint(strings.Count(content[:candidate], "\n") + 1)
			d := max(line, *args.Context) - min(line, *args.Context)
			if d < distance {
				distance = d
				index = candidate
				ambiguous = false
			} else if d == distance {
				ambiguous = true
			}
		}
		if ambiguous {
			candidates := make([]string, len(matches))
			for i, candidate := range matches {
				candidates[i] = fmt.Sprintf("occurrence %d at line %d", i+1, strings.Count(content[:candidate], "\n")+1)
			}
			return "", fmt.Errorf("Context line %d is ambiguous: %s", *args.Context, strings.Join(candidates, "; "))
		}
	default:
		if len(matches) == 0 {
			return "", fmt.Errorf("No match found in %s", args.Path)
		}
		if len(matches) > 1 {
			return "", fmt.Errorf("Multiple matches in %s; specify --occurrence or --context", args.Path)
		}
		index = matches[0]
	}
	if args.Old != args.New {
		updated := content[:index] + args.New + content[index+len(args.Old):]
		if err := ctx.Err(); err != nil {
			return "", err
		}
		if recording != nil {
			recording.Track(ctx, path)
		}
		if err := atomicWrite(ctx, path, []byte(updated), false); err != nil {
			return "", err
		}
	}
	return fmt.Sprintf("Edited %s\n", args.Path), nil
}
