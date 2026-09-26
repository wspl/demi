package shell_test

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/shell"
	"github.com/wspl/demi/internal/toolctx"
)

// The fake utilities below stand in for the standard utilities, which other
// work packages implement. Each does the least its tests need, through the
// toolctx.Invocation only.
var fakeUtilities = map[string]toolctx.Utility{
	"cat":    fakeCat,
	"tee":    fakeTee,
	"sleep":  fakeSleep,
	"wc":     fakeWc,
	"tr":     fakeUpper,
	"head":   fakeHead,
	"sort":   fakeSort,
	"sed":    fakeSedInPlace,
	"uniq":   fakeUniq,
	"cp":     fakeCp,
	"mv":     fakeMv,
	"touch":  fakeTouch,
	"mktemp": fakeMktemp,
	"mkdir":  fakeMkdir,
	"rm":     fakeRm,
	"xargs":  fakeXargs,
	"env":    fakeEnv,
	"getenv": fakeGetenv,
	"yes":    fakeYes,
}

func fail(inv *toolctx.Invocation, name string, err error) int {
	fmt.Fprintf(inv.Stderr, "%s: %v\n", name, err)
	return 1
}

// usage answers --help on the invocation's standard output.
func usage(inv *toolctx.Invocation, args []string) bool {
	if !slices.Contains(args[1:], "--help") {
		return false
	}
	fmt.Fprintf(inv.Stdout, "Usage: %s [OPTION]...\n", args[0])
	return true
}

// fakeCat copies its files, or standard input, to standard output.
func fakeCat(inv *toolctx.Invocation, args []string) int {
	if usage(inv, args) {
		return 0
	}
	if len(args) == 1 {
		args = append(args, "-")
	}
	for _, name := range args[1:] {
		var in io.Reader = inv.Stdin
		if name != "-" {
			file, err := inv.Files.Open(name)
			if err != nil {
				return fail(inv, "cat", err)
			}
			defer file.Close()
			in = file
		}
		if _, err := io.Copy(inv.Stdout, in); err != nil {
			return fail(inv, "cat", err)
		}
	}
	return 0
}

// fakeTee copies standard input to standard output and its files.
func fakeTee(inv *toolctx.Invocation, args []string) int {
	writers := []io.Writer{inv.Stdout}
	for _, name := range args[1:] {
		file, err := inv.Files.OpenFile(name, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o666)
		if err != nil {
			return fail(inv, "tee", err)
		}
		defer file.Close()
		writers = append(writers, file)
	}
	if _, err := io.Copy(io.MultiWriter(writers...), inv.Stdin); err != nil {
		return fail(inv, "tee", err)
	}
	return 0
}

// fakeSleep waits for a number of seconds, or until it is cancelled.
func fakeSleep(inv *toolctx.Invocation, args []string) int {
	seconds, err := strconv.ParseFloat(args[1], 64)
	if err != nil {
		return fail(inv, "sleep", err)
	}
	timer := time.NewTimer(time.Duration(seconds * float64(time.Second)))
	defer timer.Stop()
	select {
	case <-timer.C:
		return 0
	case <-inv.Context.Done():
		return 1
	}
}

// fakeWc prints the number of bytes on standard input, like "wc -c".
func fakeWc(inv *toolctx.Invocation, args []string) int {
	n, err := io.Copy(io.Discard, inv.Stdin)
	if err != nil {
		return fail(inv, "wc", err)
	}
	fmt.Fprintln(inv.Stdout, n)
	return 0
}

// fakeUpper upper-cases standard input, like "tr a-z A-Z".
func fakeUpper(inv *toolctx.Invocation, args []string) int {
	data, err := io.ReadAll(inv.Stdin)
	if err != nil {
		return fail(inv, "tr", err)
	}
	inv.Stdout.Write(bytes.ToUpper(data))
	return 0
}

// fakeHead copies the first line of standard input, like "head -n1".
func fakeHead(inv *toolctx.Invocation, args []string) int {
	line, err := bufio.NewReader(inv.Stdin).ReadString('\n')
	if err != nil && err != io.EOF {
		return fail(inv, "head", err)
	}
	io.WriteString(inv.Stdout, line)
	return 0
}

// fakeYes writes "y" lines until writing fails.
func fakeYes(inv *toolctx.Invocation, args []string) int {
	for {
		if _, err := io.WriteString(inv.Stdout, "y\n"); err != nil {
			return 1
		}
	}
}

func readLines(inv *toolctx.Invocation, name string) ([]string, error) {
	var in io.Reader = inv.Stdin
	if name != "-" {
		file, err := inv.Files.Open(name)
		if err != nil {
			return nil, err
		}
		defer file.Close()
		in = file
	}
	data, err := io.ReadAll(in)
	if err != nil {
		return nil, err
	}
	return strings.SplitAfter(strings.TrimSuffix(string(data), "\n"), "\n"), nil
}

func writeFile(inv *toolctx.Invocation, name, contents string) error {
	file, err := inv.Files.OpenFile(name, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o666)
	if err != nil {
		return err
	}
	_, err = io.WriteString(file, contents)
	return errors.Join(err, file.Close())
}

// fakeSort sorts the lines of a file, like "sort FILE -o OUT".
func fakeSort(inv *toolctx.Invocation, args []string) int {
	lines, err := readLines(inv, args[1])
	if err != nil {
		return fail(inv, "sort", err)
	}
	slices.Sort(lines)
	out := strings.Join(lines, "\n") + "\n"
	if len(args) == 4 && args[2] == "-o" {
		if err := writeFile(inv, args[3], strings.ReplaceAll(out, "\n\n", "\n")); err != nil {
			return fail(inv, "sort", err)
		}
		return 0
	}
	io.WriteString(inv.Stdout, out)
	return 0
}

// fakeSedInPlace replaces text in a file through a temporary file renamed
// over it, like "sed -i s/OLD/NEW/ FILE".
func fakeSedInPlace(inv *toolctx.Invocation, args []string) int {
	parts := strings.Split(args[2], "/")
	file, err := inv.Files.Open(args[3])
	if err != nil {
		return fail(inv, "sed", err)
	}
	data, err := io.ReadAll(file)
	file.Close()
	if err != nil {
		return fail(inv, "sed", err)
	}
	temp, err := inv.Files.CreateTemp(filepath.Dir(args[3]), "sed")
	if err != nil {
		return fail(inv, "sed", err)
	}
	_, err = io.WriteString(temp, strings.Replace(string(data), parts[1], parts[2], 1))
	if err := errors.Join(err, temp.Close(), inv.Files.Rename(temp.Name(), args[3])); err != nil {
		return fail(inv, "sed", err)
	}
	return 0
}

// fakeUniq writes the distinct adjacent lines of INPUT to OUTPUT.
func fakeUniq(inv *toolctx.Invocation, args []string) int {
	lines, err := readLines(inv, args[1])
	if err != nil {
		return fail(inv, "uniq", err)
	}
	lines = slices.Compact(lines)
	if err := writeFile(inv, args[2], strings.Join(lines, "")); err != nil {
		return fail(inv, "uniq", err)
	}
	return 0
}

func fakeCp(inv *toolctx.Invocation, args []string) int {
	lines, err := readLines(inv, args[1])
	if err != nil {
		return fail(inv, "cp", err)
	}
	if err := writeFile(inv, args[2], strings.Join(lines, "")); err != nil {
		return fail(inv, "cp", err)
	}
	return 0
}

func fakeMv(inv *toolctx.Invocation, args []string) int {
	if err := inv.Files.Rename(args[1], args[2]); err != nil {
		return fail(inv, "mv", err)
	}
	return 0
}

func fakeTouch(inv *toolctx.Invocation, args []string) int {
	file, err := inv.Files.OpenFile(args[1], os.O_WRONLY|os.O_CREATE, 0o666)
	if err != nil {
		return fail(inv, "touch", err)
	}
	file.Close()
	return 0
}

func fakeMktemp(inv *toolctx.Invocation, args []string) int {
	file, err := inv.Files.CreateTemp("", "tmp.")
	if err != nil {
		return fail(inv, "mktemp", err)
	}
	file.Close()
	fmt.Fprintln(inv.Stdout, file.Name())
	return 0
}

func fakeMkdir(inv *toolctx.Invocation, args []string) int {
	for _, name := range args[1:] {
		if err := inv.Files.MkdirAll(name, 0o777); err != nil {
			return fail(inv, "mkdir", err)
		}
	}
	return 0
}

func fakeRm(inv *toolctx.Invocation, args []string) int {
	for _, name := range args[1:] {
		if err := inv.Files.Remove(name); err != nil {
			return fail(inv, "rm", err)
		}
	}
	return 0
}

// fakeXargs runs its arguments followed by the words on standard input.
func fakeXargs(inv *toolctx.Invocation, args []string) int {
	data, err := io.ReadAll(inv.Stdin)
	if err != nil {
		return fail(inv, "xargs", err)
	}
	command := append(slices.Clone(args[1:]), strings.Fields(string(data))...)
	code, err := inv.Run(inv.Context, toolctx.Command{Args: command})
	if err != nil {
		return fail(inv, "xargs", err)
	}
	return code
}

// fakeEnv runs a command with NAME=VALUE arguments added to the environment.
func fakeEnv(inv *toolctx.Invocation, args []string) int {
	env := map[string]string{}
	inv.Env.Each(func(name, value string) bool {
		env[name] = value
		return true
	})
	rest := args[1:]
	for len(rest) > 0 && strings.Contains(rest[0], "=") {
		name, value, _ := strings.Cut(rest[0], "=")
		env[name] = value
		rest = rest[1:]
	}
	code, err := inv.Run(inv.Context, toolctx.Command{Args: rest, Env: mapEnv(env)})
	if err != nil {
		return fail(inv, "env", err)
	}
	return code
}

// fakeGetenv prints the values of its variable names, one per line.
func fakeGetenv(inv *toolctx.Invocation, args []string) int {
	for _, name := range args[1:] {
		value, _ := inv.Env.Get(name)
		fmt.Fprintln(inv.Stdout, value)
	}
	return 0
}

// mapEnv is a toolctx.Env held in a map.
type mapEnv map[string]string

func (e mapEnv) Get(name string) (string, bool) {
	value, ok := e[name]
	return value, ok
}

func (e mapEnv) Each(fn func(name, value string) bool) {
	for _, name := range slices.Sorted(maps.Keys(e)) {
		if !fn(name, e[name]) {
			return
		}
	}
}

// newShell returns a shell with the fake utilities and no system profile.
func newShell() *shell.Shell {
	return shell.New(shell.Config{Utilities: fakeUtilities, Umask: 0o022})
}

// testEnv is a small environment for jobs: PATH to the system programs and
// HOME in an empty directory, so that no user profile is read.
func testEnv(t *testing.T) []string {
	return []string{"PATH=/usr/bin:/bin", "HOME=" + t.TempDir()}
}

// outcome is what a finished job reported and wrote.
type outcome struct {
	result shell.Result
	stdout string
	stderr string
}

// collect reads a job's output until the job is done.
func collect(t *testing.T, job *shell.Job) outcome {
	t.Helper()
	var stdout, stderr bytes.Buffer
	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		io.Copy(&stdout, job.Stdout())
	}()
	go func() {
		defer wg.Done()
		io.Copy(&stderr, job.Stderr())
	}()
	select {
	case <-job.Done():
	case <-time.After(20 * time.Second):
		job.Cancel()
		t.Fatalf("job did not finish")
	}
	wg.Wait()
	return outcome{result: job.Result(), stdout: stdout.String(), stderr: stderr.String()}
}

// run runs script in dir with no input.
func run(t *testing.T, sh *shell.Shell, dir, script string) outcome {
	t.Helper()
	return runSpec(t, sh, shell.Spec{Script: script, Dir: dir, Env: testEnv(t)})
}

func runSpec(t *testing.T, sh *shell.Shell, spec shell.Spec) outcome {
	t.Helper()
	job, err := sh.Start(spec)
	if err != nil {
		t.Fatal(err)
	}
	job.Stdin().Close()
	return collect(t, job)
}

// wantExit fails the test unless the job exited with code.
func wantExit(t *testing.T, got outcome, code int) {
	t.Helper()
	if got.result.Status != shell.Exited || got.result.Code != code {
		t.Fatalf("result = %+v, want exit %d; stdout %q; stderr %q", got.result, code, got.stdout, got.stderr)
	}
}

// verifyNone fails the test if a goroutine outlives it. Every test that
// starts jobs checks this once its jobs are done.
func verifyNone(t *testing.T) {
	t.Helper()
	goleak.VerifyNone(t)
}

// openDescriptors counts the process's open file descriptors.
func openDescriptors(t *testing.T) int {
	t.Helper()
	entries, err := os.ReadDir("/proc/self/fd")
	if err != nil {
		t.Fatal(err)
	}
	return len(entries)
}

// readFile returns a file's contents, failing the test if it cannot.
func readFile(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// edit is one operation a recorder saw.
type edit struct {
	path   string
	before string
	after  string
}

// fakeRecorder records like the real recorder at the job's boundary: it
// reads the contents before and after each operation, while the operation
// runs under its lock, and checks that it is only given regular files or
// absent paths.
type fakeRecorder struct {
	t     *testing.T
	lock  *sync.Mutex // shared by the recorders of one installation
	mu    sync.Mutex
	edits []edit
}

func newRecorder(t *testing.T, lock *sync.Mutex) *fakeRecorder {
	return &fakeRecorder{t: t, lock: lock}
}

func (r *fakeRecorder) Record(path string, op func()) {
	if !filepath.IsAbs(path) {
		r.t.Errorf("recorded path %q is not absolute", path)
	}
	r.lock.Lock()
	defer r.lock.Unlock()
	info, err := os.Stat(path)
	if err == nil && !info.Mode().IsRegular() {
		r.t.Errorf("recorded %s, which is not a regular file", path)
	}
	before, _ := os.ReadFile(path)
	op()
	after, _ := os.ReadFile(path)
	r.mu.Lock()
	r.edits = append(r.edits, edit{path: path, before: string(before), after: string(after)})
	r.mu.Unlock()
}

// files returns the recorded paths that changed, in first-change order,
// with their latest contents.
func (r *fakeRecorder) files() ([]string, map[string]string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	var order []string
	latest := map[string]string{}
	for _, e := range r.edits {
		if e.before == e.after {
			continue
		}
		if _, ok := latest[e.path]; !ok {
			order = append(order, e.path)
		}
		latest[e.path] = e.after
	}
	return order, latest
}

// firstBefore returns the contents a path had before the first change.
func (r *fakeRecorder) firstBefore(path string) string {
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, e := range r.edits {
		if e.path == path && e.before != e.after {
			return e.before
		}
	}
	return ""
}

// fakeDispatcher runs declared roots by answering with what it received.
type fakeDispatcher struct {
	roots map[string]bool
	mu    sync.Mutex
	calls []toolctx.Command
}

func (d *fakeDispatcher) Declared(name string) bool { return d.roots[name] }

func (d *fakeDispatcher) Dispatch(ctx context.Context, cmd toolctx.Command) (int, error) {
	d.mu.Lock()
	d.calls = append(d.calls, cmd)
	d.mu.Unlock()
	input, err := io.ReadAll(cmd.Stdin)
	if err != nil {
		return 1, err
	}
	value, _ := cmd.Env.Get("DECLARED")
	fmt.Fprintf(cmd.Stdout, "%s args=%q dir=%s env=%s input=%q\n", cmd.Args[0], cmd.Args[1:], cmd.Dir, value, input)
	return 3, nil
}
