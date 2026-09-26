package text_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/toolctx/toolctxtest"
	"github.com/wspl/demi/internal/tools/text"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

func TestCat(t *testing.T)  { checkCases(t, catCases) }
func TestHead(t *testing.T) { checkCases(t, headCases) }
func TestTail(t *testing.T) { checkCases(t, tailCases) }
func TestWc(t *testing.T)   { checkCases(t, wcCases) }
func TestTee(t *testing.T)  { checkCases(t, teeCases) }
func TestSort(t *testing.T) { checkCases(t, sortCases) }
func TestUniq(t *testing.T) { checkCases(t, uniqCases) }
func TestCut(t *testing.T)  { checkCases(t, cutCases) }
func TestTr(t *testing.T)   { checkCases(t, trCases) }

// newRunner returns a runner on a fresh directory with the corpus
// environment.
func newRunner(t *testing.T) *toolctxtest.Runner {
	t.Helper()
	dir := t.TempDir()
	return &toolctxtest.Runner{Dir: dir, Env: toolctxtest.Env{"LC_ALL": "C", "HOME": dir}}
}

// Every utility prints its usage to its own standard output and succeeds.
func TestHelpAndVersion(t *testing.T) {
	runner := newRunner(t)
	for name, utility := range text.Utilities {
		for _, option := range []string{"--help", "--version"} {
			result := runner.Run(context.Background(), utility, []string{name, option, "--no-such-option"}, "")
			if result.Code != 0 || result.Stderr != "" {
				t.Errorf("%s %s: %+v", name, option, result)
			}
			if option == "--help" && !strings.HasPrefix(result.Stdout, "Usage: "+name+" ") {
				t.Errorf("%s --help: %q", name, result.Stdout)
			}
		}
	}
}

// wc -c counts every byte of a regular file, whatever its size relative
// to the page size, and the total line is named "total" (ledger L6).
func TestWcCountsEveryByteOfPageSizedFiles(t *testing.T) {
	runner := newRunner(t)
	sizes := []int{0, 4096, 65536, 307200}
	args := []string{"wc", "-c"}
	var want strings.Builder
	total := 0
	for _, size := range sizes {
		name := fmt.Sprintf("f%d", size)
		if err := os.WriteFile(filepath.Join(runner.Dir, name), bytes.Repeat([]byte("x"), size), 0o644); err != nil {
			t.Fatal(err)
		}
		args = append(args, name)
		fmt.Fprintf(&want, "%6d %s\n", size, name)
		total += size
	}
	fmt.Fprintf(&want, "%6d total\n", total)

	result := runner.Run(context.Background(), text.Utilities["wc"], args, "")

	if result.Code != 0 || result.Stdout != want.String() {
		t.Fatalf("result = %+v, want %q", result, want.String())
	}
	for _, size := range sizes {
		name := fmt.Sprintf("f%d", size)
		result := runner.Run(context.Background(), text.Utilities["wc"], []string{"wc", "-c", name}, "")
		if want := fmt.Sprintf("%d %s\n", size, name); result.Stdout != want {
			t.Errorf("wc -c %s = %q, want %q", name, result.Stdout, want)
		}
	}
}

// recordingFiles counts the opens for writing that go through Files.
type recordingFiles struct {
	toolctxtest.Files
	mu     sync.Mutex
	writes []string
}

func (f *recordingFiles) OpenFile(name string, flag int, perm fs.FileMode) (toolctx.File, error) {
	if flag&(os.O_WRONLY|os.O_RDWR) != 0 {
		f.mu.Lock()
		f.writes = append(f.writes, name)
		f.mu.Unlock()
	}
	return f.Files.OpenFile(name, flag, perm)
}

// invocation builds an invocation on dir with the given files and streams.
func invocation(ctx context.Context, dir string, files toolctx.Files, stdin io.Reader, stdout, stderr io.Writer, umask fs.FileMode) *toolctx.Invocation {
	return &toolctx.Invocation{
		Context: ctx,
		Dir:     dir,
		Env:     toolctxtest.Env{"LC_ALL": "C"},
		Stdin:   stdin,
		Stdout:  stdout,
		Stderr:  stderr,
		Umask:   umask,
		Files:   files,
	}
}

// tee, sort -o and uniq's OUTPUT write through the invocation's Files,
// resolved against its directory, and create files with its umask.
func TestWritesGoThroughFilesWithTheUmask(t *testing.T) {
	for _, run := range []struct {
		args  []string
		stdin string
		name  string
		want  string
	}{
		{[]string{"tee", "out"}, "b\na\n", "out", "b\na\n"},
		{[]string{"sort", "-o", "out"}, "b\na\n", "out", "a\nb\n"},
		{[]string{"uniq", "-", "out"}, "a\na\n", "out", "a\n"},
	} {
		t.Run(run.args[0], func(t *testing.T) {
			dir := t.TempDir()
			files := &recordingFiles{Files: toolctxtest.Files{Dir: dir}}
			var stdout, stderr bytes.Buffer
			inv := invocation(context.Background(), dir, files, strings.NewReader(run.stdin), &stdout, &stderr, 0o027)

			code := text.Utilities[run.args[0]](inv, run.args)

			if code != 0 || stderr.Len() > 0 {
				t.Fatalf("code = %d, stderr = %q", code, stderr.String())
			}
			if len(files.writes) != 1 || files.writes[0] != run.name {
				t.Fatalf("writes through Files = %q", files.writes)
			}
			path := filepath.Join(dir, run.name)
			data, err := os.ReadFile(path)
			if err != nil || string(data) != run.want {
				t.Fatalf("%s = %q, %v", run.name, data, err)
			}
			info, err := os.Stat(path)
			if err != nil || info.Mode().Perm() != 0o640 {
				t.Fatalf("mode = %v, %v", info.Mode(), err)
			}
		})
	}
}

// Concurrent sorts with small buffers and their own temporary directories
// produce their own results and leave no temporary files.
func TestConcurrentSortsKeepTheirOwnState(t *testing.T) {
	var wg sync.WaitGroup
	for index := 0; index < 2; index++ {
		runner := newRunner(t)
		if err := os.Mkdir(filepath.Join(runner.Dir, "temporary"), 0o755); err != nil {
			t.Fatal(err)
		}
		var input, want strings.Builder
		for number := 1999; number >= 0; number-- {
			fmt.Fprintf(&input, "%04d-%d\n", number, index)
		}
		for number := 0; number < 2000; number++ {
			fmt.Fprintf(&want, "%04d-%d\n", number, index)
		}
		if err := os.WriteFile(filepath.Join(runner.Dir, "input"), []byte(input.String()), 0o644); err != nil {
			t.Fatal(err)
		}
		wg.Add(1)
		go func() {
			defer wg.Done()
			args := []string{"sort", "--parallel=2", "-S", "1K", "-T", "temporary", "input"}
			result := runner.Run(context.Background(), text.Utilities["sort"], args, "")
			if result.Code != 0 || result.Stdout != want.String() {
				t.Errorf("sort %d: code %d, stderr %q", index, result.Code, result.Stderr)
			}
			entries, err := os.ReadDir(filepath.Join(runner.Dir, "temporary"))
			if err != nil || len(entries) != 0 {
				t.Errorf("temporary = %v, %v", entries, err)
			}
		}()
	}
	wg.Wait()
}

// sort -R puts equal keys next to each other, and a random source makes
// the order repeatable.
func TestRandomSortGroupsEqualKeys(t *testing.T) {
	runner := newRunner(t)
	if err := os.WriteFile(filepath.Join(runner.Dir, "seed"), bytes.Repeat([]byte{7}, 64), 0o644); err != nil {
		t.Fatal(err)
	}
	input := "a\nb\nc\na\nb\nc\na\n"
	args := []string{"sort", "-R", "--random-source=seed"}

	first := runner.Run(context.Background(), text.Utilities["sort"], args, input)
	second := runner.Run(context.Background(), text.Utilities["sort"], args, input)

	if first.Code != 0 || first.Stdout != second.Stdout {
		t.Fatalf("first %+v, second %+v", first, second)
	}
	lines := strings.Split(strings.TrimSuffix(first.Stdout, "\n"), "\n")
	seen := map[string]bool{}
	for index, line := range lines {
		if index > 0 && lines[index-1] != line && seen[line] {
			t.Fatalf("%q is not grouped in %q", line, lines)
		}
		seen[line] = true
	}
	if len(lines) != 7 {
		t.Fatalf("lines = %q", lines)
	}
}

// closedWriter is standard output whose reader went away.
type closedWriter struct{}

func (closedWriter) Write([]byte) (int, error) {
	return 0, io.ErrClosedPipe
}

// A utility whose reader went away ends quietly with the SIGPIPE status,
// as a GNU utility killed by SIGPIPE.
func TestBrokenPipeEndsQuietly(t *testing.T) {
	for name, utility := range text.Utilities {
		args := []string{name}
		switch name {
		case "tr":
			args = append(args, "a", "b")
		case "cut":
			args = append(args, "-b1")
		}
		t.Run(name, func(t *testing.T) {
			dir := t.TempDir()
			var stderr bytes.Buffer
			inv := invocation(context.Background(), dir, toolctxtest.Files{Dir: dir}, strings.NewReader("line\n"), closedWriter{}, &stderr, 0o022)

			code := utility(inv, args)

			if code != 128+13 || stderr.Len() > 0 {
				t.Fatalf("code = %d, stderr = %q", code, stderr.String())
			}
		})
	}
}

// blockingReader is standard input that has no data until the job is
// cancelled, and then fails as a job's streams do.
type blockingReader struct {
	ctx context.Context
}

func (r blockingReader) Read([]byte) (int, error) {
	<-r.ctx.Done()
	return 0, r.ctx.Err()
}

// Every utility blocked on its input returns once the job is cancelled.
func TestCancellationEndsABlockedUtility(t *testing.T) {
	for name, utility := range text.Utilities {
		args := []string{name}
		switch name {
		case "tr":
			args = append(args, "a", "b")
		case "cut":
			args = append(args, "-b1")
		}
		t.Run(name, func(t *testing.T) {
			dir := t.TempDir()
			ctx, cancel := context.WithCancel(context.Background())
			var stdout, stderr bytes.Buffer
			inv := invocation(ctx, dir, toolctxtest.Files{Dir: dir}, blockingReader{ctx}, &stdout, &stderr, 0o022)
			done := make(chan int)
			go func() {
				done <- utility(inv, args)
			}()

			cancel()

			select {
			case <-done:
			case <-time.After(5 * time.Second):
				t.Fatal("utility did not return after cancellation")
			}
		})
	}
}

// syncBuffer is standard output that a test reads while tail writes it.
type syncBuffer struct {
	mu     sync.Mutex
	buffer bytes.Buffer
}

func (b *syncBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buffer.Write(p)
}

func (b *syncBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buffer.String()
}

// waitFor polls until condition holds or fails the test.
func waitFor(t *testing.T, what string, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for !condition() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for %s", what)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// followTail starts tail with args on dir and returns its output streams,
// a cancel function and the channel its status arrives on.
func followTail(t *testing.T, dir string, args []string) (*syncBuffer, *syncBuffer, context.CancelFunc, chan int) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	stdout := &syncBuffer{}
	stderr := &syncBuffer{}
	inv := invocation(ctx, dir, toolctxtest.Files{Dir: dir}, strings.NewReader(""), stdout, stderr, 0o022)
	done := make(chan int, 1)
	go func() {
		done <- text.Utilities["tail"](inv, args)
	}()
	t.Cleanup(func() {
		cancel()
		<-done
	})
	return stdout, stderr, cancel, done
}

func appendFile(t *testing.T, path, data string) {
	t.Helper()
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND|os.O_CREATE, 0o644)
	if err != nil {
		t.Fatal(err)
	}
	_, err = file.WriteString(data)
	closeErr := file.Close()
	if err != nil || closeErr != nil {
		t.Fatal(err, closeErr)
	}
}

// tail -f prints what is appended, notices truncation, and ends when the
// job is cancelled.
func TestTailFollowsADescriptorUntilCancelled(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "log")
	appendFile(t, path, "1\n2\n")
	stdout, stderr, cancel, done := followTail(t, dir, []string{"tail", "-n1", "-s", "0.01", "-f", "log"})

	waitFor(t, "the last line", func() bool { return stdout.String() == "2\n" })
	appendFile(t, path, "3\n")
	waitFor(t, "the appended line", func() bool { return stdout.String() == "2\n3\n" })
	if err := os.WriteFile(path, []byte("new\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "the truncated file", func() bool { return stdout.String() == "2\n3\nnew\n" })
	if stderr.String() != "tail: log: file truncated\n" {
		t.Fatalf("stderr = %q", stderr.String())
	}
	cancel()
	select {
	case <-done:
		done <- 0
	case <-time.After(5 * time.Second):
		t.Fatal("tail -f did not end at cancellation")
	}
}

// tail -F waits for a missing file, and follows a new file that replaces
// the followed one.
func TestTailFollowsANameThroughReplacement(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "log")
	stdout, stderr, _, _ := followTail(t, dir, []string{"tail", "-s", "0.01", "-F", "log"})

	waitFor(t, "the missing file", func() bool { return stderr.String() != "" })
	appendFile(t, path, "first\n")
	waitFor(t, "the new file", func() bool { return stdout.String() == "first\n" })
	replacement := filepath.Join(dir, "next")
	appendFile(t, replacement, "second\n")
	if err := os.Rename(replacement, path); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "the replacing file", func() bool { return stdout.String() == "first\nsecond\n" })
	want := "tail: cannot open 'log' for reading: No such file or directory\n" +
		"tail: 'log' has appeared;  following new file\n" +
		"tail: 'log' has been replaced;  following new file\n"
	if stderr.String() != want {
		t.Fatalf("stderr = %q", stderr.String())
	}
}

// tail -f of files that cannot be followed ends at once.
func TestTailFollowWithNothingToFollowEnds(t *testing.T) {
	runner := newRunner(t)

	result := runner.Run(context.Background(), text.Utilities["tail"], []string{"tail", "-f", "missing"}, "")

	want := "tail: cannot open 'missing' for reading: No such file or directory\ntail: no files remaining\n"
	if result.Code != 1 || result.Stderr != want {
		t.Fatalf("result = %+v", result)
	}
}

// A utility that is never given a relative path resolves it against its
// invocation's directory, not the process's.
func TestPathsResolveAgainstTheInvocationDirectory(t *testing.T) {
	first := newRunner(t)
	second := newRunner(t)
	for index, runner := range []*toolctxtest.Runner{first, second} {
		if err := os.WriteFile(filepath.Join(runner.Dir, "file"), []byte(strings.Repeat("7", index+1)+"\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	for name := range text.Utilities {
		args := []string{name, "file"}
		switch name {
		case "tr":
			continue
		case "cut":
			args = []string{name, "-b1", "file"}
		case "tee":
			args = []string{name, "copy"}
		}
		for index, runner := range []*toolctxtest.Runner{first, second} {
			stdin := ""
			if name == "tee" {
				stdin = strings.Repeat("7", index+1) + "\n"
			}
			result := runner.Run(context.Background(), text.Utilities[name], args, stdin)
			want := strings.Repeat("7", index+1) + "\n"
			switch name {
			case "wc":
				want = fmt.Sprintf("1 1 %d file\n", index+2)
			case "cut":
				want = "7\n"
			}
			if result.Code != 0 || result.Stdout != want {
				t.Errorf("%s in directory %d: %+v", name, index, result)
			}
		}
	}
	if _, err := os.Stat("copy"); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("tee wrote into the process directory: %v", err)
	}
}
