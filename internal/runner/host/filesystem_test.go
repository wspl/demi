package host

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/runnerproto"
)

// fsResult checks the public wire response after a filesystem operation.
func fsResult(t *testing.T, out <-chan []byte, err error) runnerproto.FSResult {
	t.Helper()
	if err != nil {
		t.Fatal(err)
	}
	frame := receiveFrame(t, out)
	result, ok := frame.(*runnerproto.FSOK)
	if !ok {
		t.Fatalf("%#v", frame)
	}
	return result.Result
}

func TestFilesystemWirePreservesBinaryDatesLinksAndErrors(t *testing.T) {
	root := t.TempDir()
	writeFixture(t, root, "nested/a", string([]byte{0, 255, 128, 10}))
	s, out := testService(t, root, MaxFiles)
	cwd := filepath.Join(root, "nested")
	value := fsResult(t, out, s.Stat(t.Context(), runnerproto.FSStat{ID: "test", Path: "a", CWD: &cwd}))
	stat, ok := value.(*runnerproto.FSStatResult)
	if !ok || !stat.Value.IsFile {
		t.Fatal(value)
	}
	fsResult(
		t,
		out,
		s.Utimes(
			t.Context(),
			runnerproto.FSUtimes{ID: "test", Path: "nested/a", Atime: 1234567890123, Mtime: 1234567890123},
		),
	)
	value = fsResult(t, out, s.Stat(t.Context(), runnerproto.FSStat{ID: "test", Path: "nested/a"}))
	stat, ok = value.(*runnerproto.FSStatResult)
	if !ok || !stat.Value.IsFile || stat.Value.Mtime != 1234567890123 || stat.Value.Size != 4 {
		t.Fatal(value)
	}
	preEpoch := time.Unix(-1, 998500000)
	if err := os.Chtimes(filepath.Join(root, "nested/a"), preEpoch, preEpoch); err != nil {
		t.Fatal(err)
	}
	value = fsResult(t, out, s.Stat(t.Context(), runnerproto.FSStat{ID: "test", Path: "nested/a"}))
	stat, ok = value.(*runnerproto.FSStatResult)
	if !ok || stat.Value.Mtime != -1 {
		t.Fatalf("pre-epoch milliseconds: %+v", value)
	}
	fsResult(t, out, s.Link(t.Context(), runnerproto.FSLink{ID: "test", Path: "hard", ExistingPath: "nested/a"}))
	fsResult(t, out, s.Symlink(t.Context(), runnerproto.FSSymlink{ID: "test", Path: "symbolic", Target: "nested/a"}))
	value = fsResult(t, out, s.Readlink(t.Context(), runnerproto.FSReadlink{ID: "test", Path: "symbolic"}))
	link, ok := value.(*runnerproto.FSReadlinkResult)
	if !ok || link.Value != "nested/a" {
		t.Fatal(value)
	}
	value = fsResult(t, out, s.Lstat(t.Context(), runnerproto.FSLstat{ID: "test", Path: "symbolic"}))
	lstat, ok := value.(*runnerproto.FSLstatResult)
	if !ok || !lstat.Value.IsSymbolicLink {
		t.Fatal(value)
	}
	yes := true
	fsResult(
		t,
		out,
		s.Cp(t.Context(), runnerproto.FSCp{ID: "test", Path: "nested", Destination: "copied", Recursive: &yes}),
	)
	fsResult(t, out, s.Mv(t.Context(), runnerproto.FSMv{ID: "test", Path: "copied", Destination: "moved"}))
	data, err := os.ReadFile(filepath.Join(root, "moved/a"))
	if err != nil || string(data) != string([]byte{0, 255, 128, 10}) {
		t.Fatalf("%v %v", data, err)
	}
	fsResult(t, out, s.Rm(t.Context(), runnerproto.FSRm{ID: "test", Path: "moved", Recursive: &yes}))
	if err := s.Stat(t.Context(), runnerproto.FSStat{ID: "test", Path: "missing"}); err != nil {
		t.Fatal(err)
	}
	failure, ok := receiveFrame(t, out).(*runnerproto.FSError)
	if !ok || failure.Code == nil || *failure.Code != "ENOENT" {
		t.Fatal(failure)
	}
	// Every remaining metadata operation is exercised through the same boundary.
	fsResult(t, out, s.Mkdir(t.Context(), runnerproto.FSMkdir{ID: "test", Path: "created"}))
	fsResult(t, out, s.Chmod(t.Context(), runnerproto.FSChmod{ID: "test", Path: "hard", Mode: 0o600}))
	fsResult(t, out, s.Realpath(t.Context(), runnerproto.FSRealpath{ID: "test", Path: "symbolic"}))
	fsResult(t, out, s.Readdir(t.Context(), runnerproto.FSReaddir{ID: "test", Path: "."}))
	if err := os.Remove(filepath.Join(root, "nested/a")); err != nil {
		t.Fatal(err)
	}
	value = fsResult(t, out, s.Exists(t.Context(), runnerproto.FSExists{ID: "test", Path: "symbolic"}))
	exists, ok := value.(*runnerproto.FSExistsResult)
	if !ok || !exists.Value {
		t.Fatal(value)
	}
}

func TestFilesystemRequestsWait(t *testing.T) {
	root := t.TempDir()
	writeFixture(t, root, "entry", "data")
	s, out := testService(t, root, MaxFiles)
	var jobs sync.WaitGroup
	failures := make(chan error, 500)
	for i := range 500 {
		jobs.Go(func() {
			failures <- s.Readdir(t.Context(), runnerproto.FSReaddir{ID: fmt.Sprint(i), Path: "."})
		})
	}
	jobs.Wait()
	close(failures)
	for err := range failures {
		if err != nil {
			t.Fatal(err)
		}
	}
	for range 500 {
		fsResult(t, out, nil)
	}
	if err := s.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
}
