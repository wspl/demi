package interp_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"testing/synctest"
	"time"
)

// Fork regressions cost milliseconds; deadlines only bound hangs.
func TestJobJoin(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
		defer cancel()
		entered, release, complete := make(chan struct{}), make(chan struct{}), make(chan struct{})
		r, err := interp.New(interp.ExecHandlers(func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
			return func(ctx context.Context, args []string) error {
				if args[0] != "child" {
					return next(ctx, args)
				}
				close(entered)
				select {
				case <-release:
					close(complete)
					return nil
				case <-ctx.Done():
					return ctx.Err()
				}
			}
		}))
		if err != nil {
			t.Fatal(err)
		}
		f, err := syntax.NewParser().Parse(strings.NewReader(`(child &) & exit 7`), "")
		if err != nil {
			t.Fatal(err)
		}
		if err := r.Run(ctx, f); err != interp.ExitStatus(7) {
			t.Fatal(err)
		}
		select {
		case <-entered:
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
		joined := make(chan struct{})
		go func() { r.WaitBackground(); close(joined) }()
		synctest.Wait()
		select {
		case <-joined:
			t.Fatal("join returned while a nested command was blocked")
		default:
		}
		close(release)
		synctest.Wait()
		<-joined
		select {
		case <-complete:
		default:
			t.Fatal("job returned before nested background work")
		}
	})
}

func TestExecOptions(t *testing.T) {
	var out bytes.Buffer
	r, err := interp.New(interp.StdIO(nil, &out, &out), interp.ExecHandlers(func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
		return func(ctx context.Context, args []string) error {
			hc := interp.HandlerCtx(ctx)
			if args[0] != "program" || hc.Argv0 == nil || *hc.Argv0 != "custom" || !hc.ClearEnv {
				t.Errorf("exec options: %+v %v", hc, args)
			}
			return interp.ExitStatus(7)
		}
	}))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`exec -c -l -a custom program; echo unreachable`), "")
	if err != nil {
		t.Fatal(err)
	}
	if err := r.Run(t.Context(), f); err != interp.ExitStatus(7) {
		t.Fatal(err)
	}
	if out.Len() != 0 {
		t.Fatal(out.String())
	}
}

func TestProcessSubstitutionCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	r, err := interp.New(interp.ExecHandlers(func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
		return func(ctx context.Context, args []string) error {
			if args[0] == "cancel" {
				cancel()
				return ctx.Err()
			}
			return next(ctx, args)
		}
	}))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`cancel <(echo unopened) >(cat)`), "")
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { err := r.Run(ctx, f); r.WaitBackground(); done <- err }()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("FIFO cancellation did not release job")
	}
}

func TestExtraDescriptors(t *testing.T) {
	var out bytes.Buffer
	r, err := interp.New(interp.Dir(t.TempDir()), interp.StdIO(nil, &out, &out))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`exec 3<>data; printf hello >&3; exec 3<&-; exec 4<data; cat <&4; (printf world >&5) 5>>data; cat data`), "")
	if err != nil {
		t.Fatal(err)
	}
	if err := r.Run(t.Context(), f); err != nil {
		t.Fatal(err)
	}
	r.WaitBackground()
	if got := out.String(); got != "hellohelloworld" {
		t.Fatal(got)
	}
}

// A standard descriptor opened with <> keeps both sides: fd 0 is written and
// fd 2 read, while a pipeline's own stdin still stands for fd 0 inside it.
func TestStandardDescriptorsKeepBothSides(t *testing.T) {
	var out bytes.Buffer
	r, err := interp.New(interp.Dir(t.TempDir()), interp.StdIO(nil, &out, &out))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`exec 0<>data; printf hello >&0; printf ' piped' | cat <&0; cat data; printf abc > more; { cat <&2; } 2<>more`), "")
	if err != nil {
		t.Fatal(err)
	}
	if err := r.Run(t.Context(), f); err != nil {
		t.Fatal(err)
	}
	r.WaitBackground()
	if got := out.String(); got != " pipedhelloabc" {
		t.Fatal(got)
	}
}

func TestReadCancellationAfterChild(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	input, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer input.Close()
	defer writer.Close()
	entered := make(chan struct{})
	r, err := interp.New(interp.StdIO(input, nil, nil), interp.CallHandler(func(ctx context.Context, args []string) ([]string, error) {
		if args[0] == "read" {
			close(entered)
		}
		return args, nil
	}))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`sh -c :; read value`), "")
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { done <- r.Run(ctx, f) }()
	select {
	case <-entered:
		cancel()
	case <-time.After(5 * time.Second):
		t.Fatal("read was not entered")
	}
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("read did not cancel after child inherited stdin")
	}
}

func TestNestedCapturedOutput(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var out bytes.Buffer
		release := make(chan struct{})
		r, err := interp.New(interp.StdIO(nil, &out, &out), interp.ExecHandlers(func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
			return func(ctx context.Context, args []string) error {
				if args[0] != "capture" {
					return next(ctx, args)
				}
				<-release
				_, err := fmt.Fprint(interp.HandlerCtx(ctx).Stdout, "captured")
				return err
			}
		}))
		if err != nil {
			t.Fatal(err)
		}
		file, err := syntax.NewParser().Parse(strings.NewReader(`printf '%s' "$( (capture &) & )"`), "")
		if err != nil {
			t.Fatal(err)
		}
		done := make(chan error, 1)
		go func() { err := r.Run(t.Context(), file); r.WaitBackground(); done <- err }()
		synctest.Wait()
		close(release)
		synctest.Wait()
		if err := <-done; err != nil {
			t.Fatal(err)
		}
		if out.String() != "captured" {
			t.Fatal(out.String())
		}
	})
}

func TestBackgroundPipelineOutput(t *testing.T) {
	var out bytes.Buffer
	r, err := interp.New(interp.StdIO(nil, &out, &out))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`{ (echo nested &) & } | cat`), "")
	if err != nil {
		t.Fatal(err)
	}
	if err := r.Run(t.Context(), f); err != nil {
		t.Fatal(err)
	}
	r.WaitBackground()
	if out.String() != "nested\n" {
		t.Fatal(out.String())
	}
}

func TestFunctionAwareMiddleware(t *testing.T) {
	var out bytes.Buffer
	r, err := interp.New(interp.StdIO(nil, &out, &out), interp.CallHandler(func(ctx context.Context, args []string) ([]string, error) {
		if args[0] == "kill" && !interp.HandlerCtx(ctx).IsFunction(args[0]) {
			return []string{"echo", "guarded"}, nil
		}
		return args, nil
	}))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`kill; kill(){ echo function; }; kill`), "")
	if err != nil {
		t.Fatal(err)
	}
	if err := r.Run(t.Context(), f); err != nil {
		t.Fatal(err)
	}
	if out.String() != "guarded\nfunction\n" {
		t.Fatal(out.String())
	}
}

func TestPipeOwnershipHook(t *testing.T) {
	denied := errors.New("job refused pipe allocation")
	for _, script := range []string{`echo x | cat`, "cat <<EOF\nx\nEOF", `cat <<< x`} {
		r, err := interp.New(interp.PipeHandler(func(context.Context) (*os.File, *os.File, error) { return nil, nil, denied }))
		if err != nil {
			t.Fatal(err)
		}
		f, err := syntax.NewParser().Parse(strings.NewReader(script), "")
		if err != nil {
			t.Fatal(err)
		}
		// Pipelines report allocation failure through Run; redirections use the
		// shell's redirection failure status. Both must refuse execution.
		if err := r.Run(t.Context(), f); err == nil {
			t.Fatalf("executed without an owned pipe: %s", script)
		}
		r.WaitBackground()
	}
}

// recordedFile stands for a recorder's wrapper: not an *os.File itself, it
// exposes the file it wraps (patch 6).
type recordedFile struct{ file *os.File }

func (f recordedFile) Read(p []byte) (int, error)  { return f.file.Read(p) }
func (f recordedFile) Write(p []byte) (int, error) { return f.file.Write(p) }
func (f recordedFile) Close() error                { return f.file.Close() }
func (f recordedFile) FileHandle() *os.File        { return f.file }

// A recorded file redirected to a program's input reaches it as the file
// itself, not as a copy through a pipe (patch 6).
func TestRecordedFileInputIsNative(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("reads the child's descriptor through /proc")
	}
	dir := t.TempDir()
	data := filepath.Join(dir, "data")
	if err := os.WriteFile(data, []byte("input"), 0o600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	r, err := interp.New(interp.Dir(dir), interp.StdIO(nil, &out, &out), interp.OpenHandler(func(ctx context.Context, path string, flag int, perm os.FileMode) (io.ReadWriteCloser, error) {
		file, err := os.OpenFile(filepath.Join(dir, path), flag, perm)
		if err != nil {
			return nil, err
		}
		return recordedFile{file}, nil
	}))
	if err != nil {
		t.Fatal(err)
	}
	f, err := syntax.NewParser().Parse(strings.NewReader(`/bin/sh -c 'readlink /proc/self/fd/0' < data`), "")
	if err != nil {
		t.Fatal(err)
	}
	if err := r.Run(t.Context(), f); err != nil {
		t.Fatal(err)
	}
	r.WaitBackground()
	if got := strings.TrimSpace(out.String()); got != data {
		t.Fatalf("the program's input is %q, not the file %q", got, data)
	}
}
