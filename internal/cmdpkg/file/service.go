//nolint:staticcheck // Command error messages are user-visible sentences, kept byte for byte.
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
	if err := cmdsdk.CheckLaunch(args); err != nil {
		return err
	}
	return cmdsdk.ServeStdio(ctx, &service{})
}

type service struct{ mutations gates.Serial }

// Operations lists the resident service operations.
func (*service) Operations() []string { return fileop.Operations() }

func failure(err error) commandwire.Completion {
	return commandwire.Completion{
		ExitCode: 1,
		Error:    &commandwire.CommandError{Code: "command_failed", Message: err.Error()},
	}
}

// Invoke decodes a file operation and serializes mutations.
func (s *service) Invoke(
	ctx context.Context,
	invocation cmdsdk.InvocationContext[commandwire.Invocation],
) (commandwire.Completion, error) {
	op, err := fileop.Parse(invocation.Request.Operation, invocation.Request.Args)
	if err != nil {
		if errors.Is(err, fileop.ErrUnknownOperation) {
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

func read(
	ctx context.Context,
	invocation cmdsdk.InvocationContext[commandwire.Invocation],
	name string,
) (commandwire.Completion, error) {
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
	parent, ok := artifacts.Parent(path)
	if !ok {
		return errors.New("File has no parent directory")
	}
	if parent == "" {
		parent = "."
	}
	if err := os.MkdirAll(parent, 0o777); err != nil {
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
	matches := editMatches(content, args.Old)
	if err := ctx.Err(); err != nil {
		return "", err
	}
	index, err := selectEditMatch(content, matches, args)
	if err != nil {
		return "", err
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

func editMatches(content, old string) []int {
	var matches []int
	for offset := 0; offset <= len(content); {
		index := strings.Index(content[offset:], old)
		if index < 0 {
			break
		}
		index += offset
		matches = append(matches, index)
		offset = index + len(old)
	}
	return matches
}

func selectEditMatch(content string, matches []int, args *fileop.EditArgs) (int, error) {
	index := 0
	switch {
	case args.Occurrence != nil:
		if *args.Occurrence > uint(len(matches)) {
			return 0, fmt.Errorf("Occurrence %d is out of range", *args.Occurrence)
		}
		index = matches[*args.Occurrence-1]
	case args.Context != nil:
		return nearestEditMatch(content, matches, *args.Context)
	default:
		if len(matches) == 0 {
			return 0, fmt.Errorf("No match found in %s", args.Path)
		}
		if len(matches) > 1 {
			return 0, fmt.Errorf("Multiple matches in %s; specify --occurrence or --context", args.Path)
		}
		index = matches[0]
	}
	return index, nil
}

func nearestEditMatch(content string, matches []int, contextLine uint) (int, error) {
	index := 0
	if len(matches) == 0 {
		return 0, errors.New("No match found")
	}
	distance := ^uint(0)
	ambiguous := false
	for _, candidate := range matches {
		line := uint(strings.Count(content[:candidate], "\n") + 1)
		d := max(line, contextLine) - min(line, contextLine)
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
		return 0, fmt.Errorf("Context line %d is ambiguous: %s", contextLine, strings.Join(candidates, "; "))
	}
	return index, nil
}
