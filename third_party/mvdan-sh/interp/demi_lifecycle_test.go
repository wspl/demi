//go:build unix

package interp_test

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"mvdan.cc/sh/v3/expand"
	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

func script(t *testing.T, src string) *syntax.File {
	t.Helper()
	f, err := syntax.NewParser().Parse(strings.NewReader(src), "scenario")
	if err != nil {
		t.Fatal(err)
	}
	return f
}

// These scenario tests use only channels and context cancellation for ordering.
// The one-second deadlines guard hangs; the usual whole test cost is <100 ms.
func TestRecursiveJoin(t *testing.T) {
	for _, src := range []string{
		`( ( record & ) & )`,
		`f() { (record &) & }; f`,
		`x=$( (record &) ); : "$x"`,
		`cat <( (record &) )`,
		`(record &) | cat`,
	} {
		t.Run(src, func(t *testing.T) {
			ctx, cancel := context.WithTimeout(context.Background(), time.Second)
			defer cancel()
			var mu sync.Mutex
			records := 0
			r, err := interp.New(interp.ExecHandlers(func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
				return func(ctx context.Context, args []string) error {
					if args[0] != "record" {
						return next(ctx, args)
					}
					mu.Lock()
					records++
					mu.Unlock()
					return nil
				}
			}))
			if err != nil {
				t.Fatal(err)
			}
			if err = r.Run(ctx, script(t, src)); err != nil {
				t.Fatal(err)
			}
			joined := make(chan struct{})
			go func() { r.Wait(); close(joined) }()
			select {
			case <-joined:
			case <-ctx.Done():
				t.Fatal("recursive join did not finish")
			}
			mu.Lock()
			defer mu.Unlock()
			if records != 1 {
				t.Fatalf("completed job lost nested command: got %d records", records)
			}
		})
	}
}

func TestRecursiveCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	started := make(chan struct{})
	stopped := make(chan struct{})
	r, err := interp.New(interp.ExecHandlers(func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
		return func(ctx context.Context, args []string) error {
			if args[0] != "hold" {
				return next(ctx, args)
			}
			close(started)
			<-ctx.Done()
			close(stopped)
			return ctx.Err()
		}
	}))
	if err != nil {
		t.Fatal(err)
	}
	if err = r.Run(ctx, script(t, `( (hold &) & )`)); err != nil {
		t.Fatal(err)
	}
	<-started
	cancel()
	r.Wait()
	select {
	case <-stopped:
	default:
		t.Fatal("job completed before its nested handler stopped")
	}
}

func TestUnusedProcessSubstitutionCancellation(t *testing.T) {
	for _, src := range []string{`: <(echo unused)`, `: >(read -r unused)`} {
		t.Run(src, func(t *testing.T) {
			dir := t.TempDir()
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			r, err := interp.New(interp.Env(expand.ListEnviron("TMPDIR=" + dir)))
			if err != nil {
				t.Fatal(err)
			}
			if err = r.Run(ctx, script(t, src)); err != nil {
				t.Fatal(err)
			}
			cancel()
			done := make(chan error, 1)
			wait := script(t, "wait")
			go func() { done <- r.Run(context.Background(), wait.Stmts[0]) }()
			select {
			case err := <-done:
				if err != nil {
					t.Fatal(err)
				}
			case <-time.After(time.Second):
				// Rescue the original library after observing the regression, so even
				// the unpatched test run leaves no blocked FIFO task or temporary path.
				paths, e := os.ReadDir(dir)
				if e != nil {
					t.Fatal(e)
				}
				for _, p := range paths {
					f, e := os.OpenFile(filepath.Join(dir, p.Name()), os.O_RDWR, 0)
					if e != nil {
						t.Fatal(e)
					}
					defer f.Close()
				}
				<-done
				t.Fatal("cancelled process substitution still waits for an unopened FIFO")
			}
			r.Wait()
			entries, err := os.ReadDir(dir)
			if err != nil {
				t.Fatal(err)
			}
			if len(entries) != 0 {
				t.Fatalf("cancelled job leaked FIFO paths: %v", entries)
			}
		})
	}
}

func TestProcessSubstitutionOutputAndEOF(t *testing.T) {
	for _, src := range []string{
		`cat <(printf 'payload\n')`,
		`cat <( (printf 'payload\n' &) )`,
		`printf 'payload\n' > >(cat)`,
	} {
		t.Run(src, func(t *testing.T) {
			ctx, cancel := context.WithTimeout(context.Background(), time.Second)
			defer cancel()
			var output bytes.Buffer
			r, err := interp.New(interp.StdIO(nil, &output, io.Discard))
			if err != nil {
				t.Fatal(err)
			}
			if err = r.Run(ctx, script(t, src)); err != nil {
				t.Fatal(err)
			}
			r.Wait()
			if output.String() != "payload\n" {
				t.Fatalf("lost output or EOF: %q", output.String())
			}
		})
	}
}

func TestRedirectionScope(t *testing.T) {
	wrappers := map[string]string{
		"top":                  "%s",
		"subshell":             "(%s)",
		"function":             "f() { %s; }; f",
		"background":           "( %s ) &",
		"nested-background":    "( ( %s ) & )",
		"command-substitution": "x=$( %s )",
		"process-substitution": "cat <( %s )",
	}
	redirects := map[string]string{"truncate": ": > %s", "append": ": >> %s", "readwrite": ": <> %s", "descriptor": "exec 3> %s"}
	for scope, wrapper := range wrappers {
		for kind, redirect := range redirects {
			t.Run(scope+"/"+kind, func(t *testing.T) {
				dir := t.TempDir()
				var recorder demiRecorder
				defer recorder.Close()
				ctx, cancel := context.WithTimeout(context.Background(), time.Second)
				defer cancel()
				r, err := interp.New(interp.Dir(dir), interp.OpenHandler(recorder.Open))
				if err != nil {
					t.Fatal(err)
				}
				src := fmt.Sprintf(wrapper, fmt.Sprintf(redirect, "edited"))
				if err = r.Run(ctx, script(t, src)); err != nil {
					t.Fatal(err)
				}
				r.Wait()
				want := []string{filepath.Join(dir, "edited")}
				if got := recorder.Paths(); !reflect.DeepEqual(got, want) {
					t.Fatalf("recorded %v; want %v", got, want)
				}
			})
		}
	}
}

func TestReadAndExternalEditsAreExcluded(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "input"), []byte("read"), 0600); err != nil {
		t.Fatal(err)
	}
	var recorder demiRecorder
	defer recorder.Close()
	r, err := interp.New(interp.Dir(dir), interp.OpenHandler(recorder.Open))
	if err != nil {
		t.Fatal(err)
	}
	err = r.Run(context.Background(), script(t, `cat < input > /dev/null; /bin/sh -c ': > external'`))
	if err != nil {
		t.Fatal(err)
	}
	r.Wait()
	if got := recorder.Paths(); len(got) != 0 {
		t.Fatalf("recorded out-of-scope files: %v", got)
	}
}

func TestProcessSubstitutionBlockedWriteCancellation(t *testing.T) {
	dir := t.TempDir()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	writing := make(chan struct{})
	peers := make(chan *os.File, 1)
	r, err := interp.New(interp.Env(expand.ListEnviron("TMPDIR="+dir)), interp.ExecHandlers(func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
		return func(ctx context.Context, args []string) error {
			switch args[0] {
			case "burst":
				close(writing)
				_, err := io.CopyN(interp.HandlerCtx(ctx).Stdout, strings.NewReader(strings.Repeat("x", 4<<20)), 4<<20)
				return err
			case "peer":
				f, err := os.Open(args[1])
				if err != nil {
					return err
				}
				peers <- f
				// Keep the peer descriptor open after cancellation and until join returns.
				// The writer must be cancelled, not merely rescued by reader EOF.
				<-ctx.Done()
				return ctx.Err()
			}
			return next(ctx, args)
		}
	}))
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	f := script(t, `peer <(burst)`)
	go func() { done <- r.Run(ctx, f) }()
	<-writing
	peer := <-peers
	defer peer.Close()
	// Observing the first byte proves the 4 MiB write has entered the FIFO.
	// Leave the rest unread so cancellation must wake its blocked writer.
	first := make([]byte, 1)
	if _, err := io.ReadFull(peer, first); err != nil {
		t.Fatal(err)
	}
	cancel()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("foreground did not cancel")
	}
	joined := make(chan struct{})
	go func() { r.Wait(); close(joined) }()
	select {
	case <-joined:
	case <-time.After(time.Second):
		t.Fatal("full FIFO did not cancel")
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 0 {
		t.Fatalf("leaked FIFO: %v", entries)
	}
}

func TestDescriptorWritesAndNonTruncatingReadwrite(t *testing.T) {
	dir := t.TempDir()
	var recorder demiRecorder
	defer recorder.Close()
	r, err := interp.New(interp.Dir(dir), interp.OpenHandler(recorder.Open))
	if err != nil {
		t.Fatal(err)
	}
	src := `printf original > kept; : <> kept; exec 3> descriptor; printf payload >&3; (printf child >&3); printf appended >> kept`
	if err = r.Run(context.Background(), script(t, src)); err != nil {
		t.Fatal(err)
	}
	r.Wait()
	for path, want := range map[string]string{"kept": "originalappended", "descriptor": "payloadchild"} {
		got, err := os.ReadFile(filepath.Join(dir, path))
		if err != nil {
			t.Fatal(err)
		}
		if string(got) != want {
			t.Fatalf("%s: got %q; want %q", path, got, want)
		}
	}
}

func TestProcessSubstitutionDoesNotLeakDescriptors(t *testing.T) {
	// No GC is used: completion must release the job's descriptors itself.
	run := func() {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		defer cancel()
		r, err := interp.New()
		if err != nil {
			t.Fatal(err)
		}
		if err = r.Run(ctx, script(t, `while IFS= read -r line; do :; done < <(printf 'line\n')`)); err != nil {
			t.Fatal(err)
		}
		r.Wait()
		if err := ctx.Err(); err != nil {
			t.Fatalf("normal EOF needed cancellation: %v", err)
		}
	}
	run()
	before, err := os.ReadDir("/dev/fd")
	if err != nil {
		t.Fatal(err)
	}
	for range 20 {
		run()
	}
	after, err := os.ReadDir("/dev/fd")
	if err != nil {
		t.Fatal(err)
	}
	if len(after) != len(before) {
		t.Fatalf("descriptors grew from %d to %d after joined jobs", len(before), len(after))
	}
}

func TestProcessSubstitutionBlockedReadCancellation(t *testing.T) {
	dir := t.TempDir()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	reading := make(chan struct{})
	peers := make(chan *os.File, 1)
	r, err := interp.New(interp.Env(expand.ListEnviron("TMPDIR="+dir)), interp.ExecHandlers(func(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
		return func(ctx context.Context, args []string) error {
			switch args[0] {
			case "drain":
				close(reading)
				_, err := io.Copy(io.Discard, interp.HandlerCtx(ctx).Stdin)
				return err
			case "peer":
				f, err := os.OpenFile(args[1], os.O_WRONLY, 0)
				if err != nil {
					return err
				}
				peers <- f
				<-ctx.Done()
				return ctx.Err()
			}
			return next(ctx, args)
		}
	}))
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	f := script(t, `peer >(drain)`)
	go func() { done <- r.Run(ctx, f) }()
	<-reading
	peer := <-peers
	defer peer.Close()
	cancel()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("foreground did not cancel")
	}
	joined := make(chan struct{})
	go func() { r.Wait(); close(joined) }()
	select {
	case <-joined:
	case <-time.After(time.Second):
		t.Fatal("empty FIFO did not cancel with writer still open")
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 0 {
		t.Fatalf("leaked FIFO: %v", entries)
	}
}

func TestNoShebangScriptSharesJobOwnershipAndRecorder(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "script"), []byte("( (printf done > marker &) & )\n"), 0700); err != nil {
		t.Fatal(err)
	}
	var recorder demiRecorder
	defer recorder.Close()
	r, err := interp.New(interp.Dir(dir), interp.OpenHandler(recorder.Open))
	if err != nil {
		t.Fatal(err)
	}
	if err = r.Run(context.Background(), script(t, "./script")); err != nil {
		t.Fatal(err)
	}
	r.Wait()
	want := []string{filepath.Join(dir, "marker")}
	if got := recorder.Paths(); !reflect.DeepEqual(got, want) {
		t.Fatalf("fallback escaped recorder: got %v; want %v", got, want)
	}
	if data, err := os.ReadFile(want[0]); err != nil || string(data) != "done" {
		t.Fatalf("fallback background write incomplete: %q, %v", data, err)
	}
}

func TestRegularFileWithFIFOPrefixStillReachesRecorder(t *testing.T) {
	dir := t.TempDir()
	var recorder demiRecorder
	defer recorder.Close()
	r, err := interp.New(interp.Dir(dir), interp.Env(expand.ListEnviron("TMPDIR="+dir)), interp.OpenHandler(recorder.Open))
	if err != nil {
		t.Fatal(err)
	}
	if err = r.Run(context.Background(), script(t, `: > "$TMPDIR/sh-interp-user-file"`)); err != nil {
		t.Fatal(err)
	}
	r.Wait()
	want := []string{filepath.Join(dir, "sh-interp-user-file")}
	if got := recorder.Paths(); !reflect.DeepEqual(got, want) {
		t.Fatalf("regular file bypassed recorder: %v", got)
	}
}
