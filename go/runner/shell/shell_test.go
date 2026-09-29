package shell_test

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/runner/shell"
)

func TestMain(m *testing.M) {
	windowsFixture()
	shell.ExecHelper()
	os.Exit(m.Run())
}

// These scenarios run real Host programs and interpreter jobs. Each costs less
// than a second without login; the profile scenario reads the system profile.
type output struct{ stdout, stderr bytes.Buffer }

func (o *output) Write(_ context.Context, stream string, p []byte) error {
	if stream == "stdout" {
		_, err := o.stdout.Write(p)
		return err
	}
	_, err := o.stderr.Write(p)
	return err
}
func options(t *testing.T) shell.Options {
	t.Helper()
	root := t.TempDir()
	return shell.Options{Dir: root, Env: map[string]string{"HOME": root, "PATH": os.Getenv("PATH")}}
}
func execute(t *testing.T, opts shell.Options, script string) (shell.Result, *output) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	out := &output{}
	opts.Output = out
	result := shell.Run(ctx, script, opts)
	if result.Err != nil || result.Signal != "" {
		t.Fatalf("Run(%q): %+v; stderr %s", script, result, out.stderr.String())
	}
	return result, out
}
func unix(t *testing.T) {
	t.Helper()
	if runtime.GOOS == "windows" {
		t.Skip("requires Unix Host programs")
	}
}

func TestStateAndPipelines(t *testing.T) {
	unix(t)
	opts := options(t)
	result, out := execute(t, opts, `mkdir a b; f() { printf '%s\n' "$1"; }; f pear > a/input; (cd a; cat input) | tr a-z A-Z; cd b; export LEAK=bad`)
	if *result.Code != 0 || result.Dir != filepath.Join(opts.Dir, "b") || out.stdout.String() != "PEAR\n" {
		t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
	}
	_, out = execute(t, opts, `printf '%s' "${LEAK-unset}"`)
	if out.stdout.String() != "unset" {
		t.Fatal(out.stdout.String())
	}
	opts.Env["DEMI_CONTEXT_ID"] = "owned"
	_, out = execute(t, opts, `export DEMI_CONTEXT_ID=forged; /bin/sh -c 'echo "$DEMI_CONTEXT_ID"'`)
	if out.stdout.String() != "owned\n" {
		t.Fatalf("external context: %q", out.stdout.String())
	}

}

func TestBackgroundAndExec(t *testing.T) {
	unix(t)
	opts := options(t)
	result, _ := execute(t, opts, `(echo completed > done) & exit 7`)
	data, err := os.ReadFile(filepath.Join(opts.Dir, "done"))
	if err != nil || string(data) != "completed\n" || *result.Code != 7 {
		t.Fatalf("%s %v %+v", data, err, result)
	}
	result, out := execute(t, opts, `(exec /bin/sh -c 'exit 3'); echo "subshell $?"; exec /bin/sh -c 'echo last; exit 7'; echo after`)
	if *result.Code != 7 || out.stdout.String() != "subshell 3\nlast\n" {
		t.Fatalf("%+v %q", result, out.stdout.String())
	}
}

func TestProcessBuiltins(t *testing.T) {
	unix(t)
	opts := options(t)
	_, out := execute(t, opts, `kill $$; echo "runner $?"; kill -s TERM 0; echo "group $?"; suspend -f; echo "suspend $?"; true & fg; echo "fg $?"`)
	if out.stdout.String() != "runner 1\ngroup 1\nsuspend 1\nfg 1\n" || strings.Count(out.stderr.String(), "a job cannot signal the runner it runs in") != 2 {
		t.Fatalf("%q %q", out.stdout.String(), out.stderr.String())
	}
	_, out = execute(t, opts, `ulimit -n 64; ulimit -n; /bin/sh -c 'ulimit -n'; ulimit -n 99999999999; echo "refused $?"; ulimit -Hn`)
	if out.stdout.String() != "64\n64\nrefused 1\n64\n" {
		t.Fatalf("%q %q", out.stdout.String(), out.stderr.String())
	}
	_, out = execute(t, opts, `ulimit -t 0; echo configured; ulimit -t`)
	if out.stdout.String() != "configured\n0\n" || out.stderr.Len() != 0 {
		t.Fatalf("zero CPU limit: %q %q", out.stdout.String(), out.stderr.String())
	}
	if err := os.WriteFile(filepath.Join(opts.Dir, "before"), nil, 0666); err != nil {
		t.Fatal(err)
	}
	_, out = execute(t, opts, `umask 077; umask; /bin/sh -c umask; umask -S; echo x > redirected; touch touched; mkdir made; (umask 022); umask`)
	if out.stdout.String() != "0077\n0077\nu=rwx,g=,o=\n0077\n" {
		t.Fatalf("%q %q", out.stdout.String(), out.stderr.String())
	}
	for name, mode := range map[string]os.FileMode{"redirected": 0600, "touched": 0600, "made": 0700} {
		info, err := os.Stat(filepath.Join(opts.Dir, name))
		if err != nil || info.Mode().Perm() != mode {
			t.Fatalf("%s: %v %v", name, info, err)
		}
	}
	if err := os.WriteFile(filepath.Join(opts.Dir, "after"), nil, 0666); err != nil {
		t.Fatal(err)
	}
	before, _ := os.Stat(filepath.Join(opts.Dir, "before"))
	after, _ := os.Stat(filepath.Join(opts.Dir, "after"))
	if before.Mode() != after.Mode() {
		t.Fatal("runner umask changed")
	}
}

func TestLoginRestoresContext(t *testing.T) {
	unix(t)
	opts := options(t)
	opts.Login = true
	opts.Env["DEMI_CONTEXT_ID"] = "owned"
	opts.Env["PATH"] = filepath.Join(opts.Dir, "aliases")
	if err := os.Mkdir(filepath.Join(opts.Dir, "elsewhere"), 0700); err != nil {
		t.Fatal(err)
	}
	for _, value := range []string{"first", "next"} {
		profile := "export FROM_PROFILE=" + value + "\nexport PATH=\"$HOME/tools\"\nexport DEMI_CONTEXT_ID=wrong\ncd \"$HOME/elsewhere\"\n"
		if err := os.WriteFile(filepath.Join(opts.Dir, ".bash_profile"), []byte(profile), 0600); err != nil {
			t.Fatal(err)
		}
		result, out := execute(t, opts, `printf '%s\n' "$FROM_PROFILE" "$DEMI_CONTEXT_ID" "$PATH"`)
		want := value + "\nowned\n" + opts.Env["PATH"] + ":" + filepath.Join(opts.Dir, "tools") + "\n"
		if result.Dir != opts.Dir || out.stdout.String() != want {
			t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
		}
	}
}

func TestLargeCompoundPipelinesAndProcessSubstitution(t *testing.T) {
	unix(t)
	opts := options(t)
	content := bytes.Repeat([]byte("x"), 262144)
	if err := os.WriteFile(filepath.Join(opts.Dir, "input"), content, 0600); err != nil {
		t.Fatal(err)
	}
	script := "producer() { cat input; }; value=$(producer | cat | cat); printf '%s\\n' \"${#value}\"; { producer; } | wc -c; (producer) | wc -c; cat <<EOF | wc -c\n$value\nEOF\ncat <<< \"$value\" | wc -c; cat input | tee >(cat > copied) > /dev/null"
	result, out := execute(t, opts, script)
	if *result.Code != 0 || strings.Join(strings.Fields(out.stdout.String()), " ") != "262144 262144 262144 262145 262145" {
		t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
	}
	data, err := os.ReadFile(filepath.Join(opts.Dir, "copied"))
	if err != nil || !bytes.Equal(data, content) {
		t.Fatalf("copied %d bytes: %v", len(data), err)
	}
}

type eventSink struct{ received chan []byte }

func (s eventSink) Write(ctx context.Context, _ string, p []byte) error {
	select {
	case s.received <- bytes.Clone(p):
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
func TestPipelineStreamsBeforeEOF(t *testing.T) {
	unix(t)
	opts := options(t)
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer writer.Close()
	opts.Stdin = reader
	sink := eventSink{make(chan []byte, 4)}
	opts.Output = sink
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	done := make(chan shell.Result, 1)
	go func() { done <- shell.Run(ctx, "cat | cat", opts) }()
	payload := []byte{0, 255, 128, 10}
	if _, err := writer.Write(payload); err != nil {
		t.Fatal(err)
	}
	var received []byte
	for len(received) < len(payload) {
		select {
		case p := <-sink.received:
			received = append(received, p...)
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	if !bytes.Equal(payload, received) {
		t.Fatal(received)
	}
	writer.Close()
	result := <-done
	if result.Code == nil || *result.Code != 0 {
		t.Fatal(result)
	}
}

func TestCancellationIsIsolated(t *testing.T) {
	unix(t)
	for _, script := range []string{`while :; do :; done`, `read value`, `cat`, `(sleep 60) & wait`, `cat <(sleep 60)`} {
		t.Run(script, func(t *testing.T) {
			opts := options(t)
			reader, writer, err := os.Pipe()
			if err != nil {
				t.Fatal(err)
			}
			defer writer.Close()
			opts.Stdin = reader
			sink := eventSink{make(chan []byte, 4)}
			opts.Output = sink
			ctx, stop := context.WithTimeout(t.Context(), 5*time.Second)
			defer stop()
			job, cancel := context.WithCancelCause(ctx)
			done := make(chan shell.Result, 1)
			go func() { done <- shell.Run(job, "echo ready; "+script, opts) }()
			select {
			case <-sink.received:
			case <-ctx.Done():
				t.Fatal(ctx.Err())
			}
			cancel(shell.CancelCause("SIGTERM"))
			select {
			case result := <-done:
				if result.Signal != "SIGTERM" || result.Code != nil {
					t.Fatal(result)
				}
			case <-ctx.Done():
				t.Fatal(ctx.Err())
			}
			result, out := execute(t, options(t), `printf '%s' $$`)
			if *result.Code != 0 || out.stdout.String() != fmt.Sprint(os.Getpid()) {
				t.Fatalf("%+v %s", result, out.stdout.String())
			}
		})
	}
}

// Two jobs run side by side; cancelling one leaves the other running, and it
// finishes with its own input and output.
// Cost: two jobs on loopback pipes; a five-second guard bounds the waits.
func TestSiblingJobKeepsRunningWhileAnotherIsCancelled(t *testing.T) {
	unix(t)
	ctx, stop := context.WithTimeout(t.Context(), 5*time.Second)
	defer stop()
	type job struct {
		writer *os.File
		sink   eventSink
		first  []byte
		done   chan shell.Result
		cancel context.CancelCauseFunc
	}
	start := func() job {
		opts := options(t)
		reader, writer, err := os.Pipe()
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { writer.Close() })
		opts.Stdin = reader
		j := job{writer: writer, sink: eventSink{make(chan []byte, 16)}, done: make(chan shell.Result, 1)}
		opts.Output = j.sink
		run, cancel := context.WithCancelCause(ctx)
		j.cancel = cancel
		go func() { j.done <- shell.Run(run, "echo ready; read value; echo \"got $value\"", opts) }()
		select {
		case j.first = <-j.sink.received:
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
		return j
	}
	cancelled, sibling := start(), start()
	cancelled.cancel(shell.CancelCause("SIGTERM"))
	select {
	case result := <-cancelled.done:
		if result.Signal != "SIGTERM" {
			t.Fatal(result)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	if _, err := sibling.writer.Write([]byte("more\n")); err != nil {
		t.Fatal(err)
	}
	select {
	case result := <-sibling.done:
		if result.Code == nil || *result.Code != 0 {
			t.Fatal(result)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	// The job has ended, so all it wrote is in the sink.
	written := string(sibling.first)
	for len(sibling.sink.received) > 0 {
		written += string(<-sibling.sink.received)
	}
	if written != "ready\ngot more\n" {
		t.Fatalf("sibling wrote %q", written)
	}
}

// A blocked sink proves that cancellation releases backpressure, with no timer
// used to decide when to cancel the job.
type blockedSink struct {
	entered chan struct{}
	once    sync.Once
}

func (s *blockedSink) Write(ctx context.Context, _ string, _ []byte) error {
	s.once.Do(func() { close(s.entered) })
	<-ctx.Done()
	return ctx.Err()
}
func TestCancellationReleasesOutput(t *testing.T) {
	opts := options(t)
	sink := &blockedSink{entered: make(chan struct{})}
	opts.Output = sink
	ctx, stop := context.WithTimeout(t.Context(), 5*time.Second)
	defer stop()
	job, cancel := context.WithCancel(ctx)
	done := make(chan shell.Result, 1)
	go func() { done <- shell.Run(job, "echo ready", opts) }()
	select {
	case <-sink.entered:
	case <-ctx.Done():
		t.Fatal("output did not start:", ctx.Err())
	}
	cancel()
	select {
	case result := <-done:
		if result.Signal != "SIGKILL" {
			t.Fatal(result)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
}

func TestRedirectionSnapshots(t *testing.T) {
	unix(t)
	opts := options(t)
	recorder, err := commandservice.NewRecorder(commandservice.EditContext{Directory: filepath.Join(opts.Dir, "changes"), Lock: filepath.Join(opts.Dir, "edits.lock")})
	if err != nil {
		t.Fatal(err)
	}
	opts.Recorder = recorder
	result, out := execute(t, opts, `printf 'one\n' > note; /bin/sh -c 'echo two' >> note; exec 3>>note; printf 'three\n' >&3; exec 3<&-; cat note`)
	if *result.Code != 0 || out.stdout.String() != "one\ntwo\nthree\n" {
		t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
	}
	files, truncated := shell.ReportEdits(recorder, func(before, after []byte) (uint64, uint64) {
		if before != nil || string(after) != "one\ntwo\nthree\n" {
			t.Errorf("snapshots: %q -> %q", before, after)
		}
		return 3, 0
	})
	if truncated || len(files) != 1 || files[0].Kind != commandservice.EditAdded || files[0].Added != 3 || len(files[0].Edits) != 1 {
		t.Fatalf("%+v truncated=%v", files, truncated)
	}
	// An unavailable snapshot is never replaced by the live file at completion.
	if err := os.Remove(*files[0].Edits[0].Modified); err != nil {
		t.Fatal(err)
	}
	files, _ = shell.ReportEdits(recorder, func([]byte, []byte) (uint64, uint64) { t.Error("counted unavailable snapshot"); return 0, 0 })
	if files[0].Edits[0].Modified != nil || files[0].Edits[0].Original != nil || files[0].Added != 0 {
		t.Fatal(files)
	}
}

func TestBuiltinFunctionsAndUsage(t *testing.T) {
	unix(t)
	_, out := execute(t, options(t), `umask() { echo function; }; umask; builtin umask 077 022; echo $?; ulimit -z; echo $?; ulimit -p 1; echo $?; command umask 089; echo $?`)
	if got := out.stdout.String(); got != "function\n2\n2\n1\n1\n" {
		t.Fatal(got)
	}
	want := "umask: 022: too many arguments\nulimit: -z: invalid option\nulimit: pipe size: cannot modify limit: Invalid argument\numask: 089: octal number out of range\n"
	if got := out.stderr.String(); got != want {
		t.Fatalf("got %q want %q", got, want)
	}
}

func TestExecAttributesAndDescriptors(t *testing.T) {
	unix(t)
	opts := options(t)
	_, out := execute(t, opts, `exec 3<>data; printf builtin >&3; sh -c 'printf child >&3'; exec 3<&-; cat data; (umask 077; exec -a name sh -c 'printf ":%s:" "$0"; touch protected'); (exec -c /usr/bin/env); echo complete`)
	if out.stdout.String() != "builtinchild:name:complete\n" || out.stderr.Len() != 0 {
		t.Fatalf("%q %q", out.stdout.String(), out.stderr.String())
	}
	info, err := os.Stat(filepath.Join(opts.Dir, "protected"))
	if err != nil || info.Mode().Perm() != 0600 {
		t.Fatalf("%v %v", info, err)
	}
}

func TestReadAfterExternalCanCancel(t *testing.T) {
	unix(t)
	opts := options(t)
	input, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer writer.Close()
	opts.Stdin = input
	sink := eventSink{make(chan []byte, 4)}
	opts.Output = sink
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	done := make(chan shell.Result, 1)
	go func() { done <- shell.Run(ctx, `sh -c :; echo ready; read line`, opts) }()
	select {
	case <-sink.received:
		cancel()
	case <-time.After(5 * time.Second):
		t.Fatal("missing ready")
	}
	select {
	case result := <-done:
		if result.Signal != "SIGKILL" {
			t.Fatal(result)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("read did not cancel")
	}
}

func TestUbuntuProfiles(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("Ubuntu profile fixture uses /usr/bin/locale-check")
	}
	if _, err := os.Stat("/usr/bin/locale-check"); err != nil {
		t.Skip("Ubuntu locale-check is not installed")
	}
	opts := options(t)
	opts.Env["PATH"] = "/usr/bin:/bin"
	for _, dir := range []string{"bin", ".local/bin", "profile.d"} {
		if err := os.MkdirAll(filepath.Join(opts.Dir, dir), 0755); err != nil {
			t.Fatal(err)
		}
	}
	for source, destination := range map[string]string{"profile": "profile", "user.profile": ".profile", "profile.d/01-locale-fix.sh": "profile.d/01-locale-fix.sh"} {
		data, err := os.ReadFile(filepath.Join("testdata/ubuntu", source))
		if err != nil {
			t.Fatal(err)
		}
		data = bytes.ReplaceAll(data, []byte("/etc/profile.d"), []byte(filepath.Join(opts.Dir, "profile.d")))
		if err := os.WriteFile(filepath.Join(opts.Dir, destination), data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(opts.Dir, ".bashrc"), []byte("echo must-not-run >&2"), 0600); err != nil {
		t.Fatal(err)
	}
	_, out := execute(t, opts, `. ./profile; . "$HOME/.profile"; printf '%s\n' "$PATH" "${BASH_VERSION-unset}" "${LC_ALL-unset}"`)
	want := opts.Dir + "/.local/bin:" + opts.Dir + "/bin:/usr/bin:/bin\nunset\nunset\n"
	if out.stdout.String() != want || out.stderr.Len() != 0 {
		t.Fatalf("stdout %q; stderr %q", out.stdout.String(), out.stderr.String())
	}
}

func TestExecUsage(t *testing.T) {
	unix(t)
	_, out := execute(t, options(t), `(exec -a); echo $?; (exec -c -c); echo $?; (exec -z); echo $?`)
	if out.stdout.String() != "2\n2\n127\n" {
		t.Fatal(out.stdout.String())
	}
	want := "error: a value is required for '-a <NAME>' but none was supplied\n\nFor more information, try '--help'.\n\n" +
		"error: the argument '-c' cannot be used multiple times\n\nUsage: exec [OPTIONS] [COMMAND]...\n\nFor more information, try '--help'.\n\nexec: -z: not found\n"
	if out.stderr.String() != want {
		t.Fatalf("got %q want %q", out.stderr.String(), want)
	}
}

func TestRetainedPipeEndsBeforeJoin(t *testing.T) {
	unix(t)
	opts := options(t)
	_, out := execute(t, opts, `exec 3> >(cat >data); (printf nested >&3) & printf root >&3; exec 3>&-; wait`)
	data, err := os.ReadFile(filepath.Join(opts.Dir, "data"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "root") || !strings.Contains(string(data), "nested") || out.stderr.Len() != 0 {
		t.Fatalf("%q %q", data, out.stderr.String())
	}
}

func TestCommandCompletionEndsDescendants(t *testing.T) {
	unix(t)
	result, out := execute(t, options(t), `sh -c 'sleep 60 & printf completed'`)
	if result.Code == nil || *result.Code != 0 || out.stdout.String() != "completed" || out.stderr.Len() != 0 {
		t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
	}
}
