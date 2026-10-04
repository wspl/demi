package host

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/runnerwire"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// gitCommand prepares reference fixtures and asks Git for the porcelain oracle.
func gitCommand(t *testing.T, root string, args ...string) string {
	t.Helper()
	command := exec.CommandContext(t.Context(), "git", append([]string{"-C", root}, args...)...)
	command.Env = append(
		os.Environ(),
		"GIT_CONFIG_GLOBAL=/dev/null",
		"GIT_CONFIG_SYSTEM=/dev/null",
		"GIT_AUTHOR_NAME=Host Test",
		"GIT_AUTHOR_EMAIL=host@example.com",
		"GIT_COMMITTER_NAME=Host Test",
		"GIT_COMMITTER_EMAIL=host@example.com",
	)
	out, err := command.CombinedOutput()
	if err != nil {
		t.Fatalf("git %v: %v: %s", args, err, out)
	}
	return string(out)
}

func writeFixture(t *testing.T, root, path, content string) {
	t.Helper()
	name := filepath.Join(root, path)
	if err := os.MkdirAll(filepath.Dir(name), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(name, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func repositoryFixture(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	gitCommand(t, root, "init", "-b", "main")
	for path, content := range map[string]string{"a.txt": "1\n2\n3\n", "b.txt": "x\ny\n", "dir/c.txt": "c\n"} {
		writeFixture(t, root, path, content)
	}
	commitFixture(t, root)
	return root
}

func commitFixture(t *testing.T, root string) {
	t.Helper()
	gitCommand(t, root, "add", "-A")
	gitCommand(t, root, "commit", "-m", "fixture")
}

func testService(t *testing.T, root string, limit int) (*Service, chan []byte) {
	t.Helper()
	output := make(chan []byte, 2048)
	service := NewWithFileLimit(t.Context(), root, nil, output, limit)
	t.Cleanup(func() {
		if err := service.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return service, output
}

func receiveFrame(t *testing.T, output <-chan []byte) runnerwire.Outbound {
	t.Helper()
	select {
	case bytes := <-output:
		frame, err := runnerwire.DecodeOutbound(bytes)
		if err != nil {
			t.Fatal(err)
		}
		return frame
	case <-t.Context().Done():
		t.Fatal(t.Context().Err())
		return nil
	}
}

func changesFixture(t *testing.T, service *Service, output <-chan []byte, root string) runnerwire.GitChanges {
	t.Helper()
	if err := service.GitChanges(t.Context(), runnerwire.GitChangesMessage{ID: "changes", Root: root}); err != nil {
		t.Fatal(err)
	}
	frame := receiveFrame(t, output)
	ok, valid := frame.(*runnerwire.GitOK)
	if !valid {
		t.Fatalf("changes: %#v", frame)
	}
	result, valid := ok.Result.(*runnerwire.GitChangesResult)
	if !valid {
		t.Fatalf("result: %#v", ok.Result)
	}
	return result.Value
}

func changeMap(changes runnerwire.GitChanges) map[string]runnerwire.GitChange {
	result := make(map[string]runnerwire.GitChange)
	for _, change := range changes.Files {
		result[change.Path] = change
	}
	return result
}

func TestDirectoryOutsideRepository(t *testing.T) {
	root := t.TempDir()
	s, out := testService(t, root, MaxFiles)
	result := changesFixture(t, s, out, root)
	if result.Repository || result.Watched || result.Head != nil || len(result.Files) != 0 || result.Truncated {
		t.Fatalf("%+v", result)
	}
}

func TestChangesJudgedAgainstHead(t *testing.T) {
	root := repositoryFixture(t)
	writeFixture(t, root, "a.txt", "1\nchanged\n3\n4\n")
	if err := os.Remove(filepath.Join(root, "b.txt")); err != nil {
		t.Fatal(err)
	}
	writeFixture(t, root, "d.txt", "new\nfile")
	gitCommand(t, root, "mv", "dir/c.txt", "dir/e.txt")
	writeFixture(t, root, "a2.txt", "temporary\n")
	gitCommand(t, root, "add", "a2.txt")
	if err := os.Remove(filepath.Join(root, "a2.txt")); err != nil {
		t.Fatal(err)
	}
	s, out := testService(t, root, MaxFiles)
	from := "dir/c.txt"
	want := []runnerwire.GitChange{
		{Path: "a.txt", Status: " M", Kind: runnerwire.ChangeKindModified, Added: 2, Removed: 1},
		{Path: "b.txt", Status: " D", Kind: runnerwire.ChangeKindDeleted, Removed: 2},
		{Path: "d.txt", Status: "??", Kind: runnerwire.ChangeKindAdded, Added: 2},
		{Path: "dir/e.txt", Status: "R ", Kind: runnerwire.ChangeKindRenamed, From: &from},
	}
	result := changesFixture(t, s, out, root)
	if !result.Repository || result.Head == nil || result.Truncated {
		t.Fatal(result)
	}
	if diff := cmp.Diff(want, result.Files); diff != "" {
		t.Fatal(diff)
	}
}

func TestEveryPathCarriesGitStatusLetters(t *testing.T) {
	root := repositoryFixture(t)
	for _, name := range []string{"s", "d", "e", "f", "t", "x.sh", "clash"} {
		writeFixture(t, root, name, name+"\n")
	}
	commitFixture(t, root)
	gitCommand(t, root, "checkout", "-b", "other")
	writeFixture(t, root, "clash", "other\n")
	commitFixture(t, root)
	gitCommand(t, root, "checkout", "main")
	writeFixture(t, root, "clash", "main\n")
	commitFixture(t, root)
	command := exec.CommandContext(
		t.Context(),
		"git",
		"-C",
		root,
		"-c",
		"user.name=Host",
		"-c",
		"user.email=host@example.com",
		"merge",
		"other",
	)
	if err := command.Run(); err == nil {
		t.Fatal("expected conflict")
	}
	writeFixture(t, root, "a.txt", "1\n2\n3\n4\n")
	writeFixture(t, root, "b.txt", "staged\n")
	gitCommand(t, root, "add", "b.txt")
	writeFixture(t, root, "s", "staged\n")
	gitCommand(t, root, "add", "s")
	writeFixture(t, root, "s", "working\n")
	if err := os.Remove(filepath.Join(root, "d")); err != nil {
		t.Fatal(err)
	}
	gitCommand(t, root, "rm", "e")
	writeFixture(t, root, "untracked", "new\n")
	writeFixture(t, root, "added", "added\n")
	gitCommand(t, root, "add", "added")
	writeFixture(t, root, "am", "added\n")
	gitCommand(t, root, "add", "am")
	writeFixture(t, root, "am", "modified\n")
	writeFixture(t, root, "intent", "intent\n")
	gitCommand(t, root, "add", "-N", "intent")
	gitCommand(t, root, "mv", "dir/c.txt", "moved")
	if err := os.Rename(filepath.Join(root, "f"), filepath.Join(root, "g")); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(filepath.Join(root, "x.sh"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(filepath.Join(root, "t")); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("a.txt", filepath.Join(root, "t")); err != nil {
		t.Fatal(err)
	}
	want := gitStatuses(t, root)
	s, out := testService(t, root, MaxFiles)
	got := map[string]string{}
	for _, change := range changesFixture(t, s, out, root).Files {
		got[change.Path] = change.Status
		if change.Path == "x.sh" &&
			(change.Kind != runnerwire.ChangeKindModified || change.Added != 0 || change.Removed != 0) {
			t.Fatal(change)
		}
	}
	if diff := cmp.Diff(want, got); diff != "" {
		t.Fatal(diff)
	}
}

func TestSubtreeRelativePaths(t *testing.T) {
	root := repositoryFixture(t)
	gitCommand(t, root, "mv", "dir/c.txt", "dir/e.txt")
	writeFixture(t, root, "dir/new.txt", "new\n")
	writeFixture(t, root, "outside", "outside\n")
	s, out := testService(t, root, MaxFiles)
	changes := changesFixture(t, s, out, filepath.Join(root, "dir"))
	want := []runnerwire.GitChange{
		{Path: "e.txt", Status: "R ", Kind: runnerwire.ChangeKindRenamed, From: new("c.txt")},
		{Path: "new.txt", Status: "??", Kind: runnerwire.ChangeKindAdded, Added: 1},
	}
	if diff := cmp.Diff(want, changes.Files); diff != "" {
		t.Fatal(diff)
	}
	data, err := s.showBlob(t.Context(), filepath.Join(root, "dir"), "c.txt")
	if err != nil || string(data) != "c\n" {
		t.Fatalf("%q %v", data, err)
	}
}

func TestListStopsAtFileLimit(t *testing.T) {
	root := repositoryFixture(t)
	for _, name := range []string{"one", "two", "three"} {
		writeFixture(t, root, name, "new\n")
	}
	s, out := testService(t, root, 2)
	result := changesFixture(t, s, out, root)
	if !result.Truncated || len(result.Files) != 2 {
		t.Fatal(result)
	}
}

func TestShowReadsCommitAndRefusesMissing(t *testing.T) {
	root := repositoryFixture(t)
	writeFixture(t, root, "a.txt", "changed\n")
	writeFixture(t, root, "untracked", "new\n")
	s, _ := testService(t, root, MaxFiles)
	data, err := s.showBlob(t.Context(), root, "a.txt")
	if err != nil || string(data) != "1\n2\n3\n" {
		t.Fatalf("%q %v", data, err)
	}
	for _, test := range []struct{ root, path, code string }{
		{root, "untracked", "ENOENT"},
		{root, "dir", "EISDIR"},
		{t.TempDir(), "a.txt", "not_repository"},
	} {
		_, err := s.showBlob(t.Context(), test.root, test.path)
		if err == nil || gitProblem(err).code != test.code {
			t.Fatalf("%s: %v", test.path, err)
		}
	}
}

func TestCancelledRequestAnswersCancelled(t *testing.T) {
	root := repositoryFixture(t)
	s, out := testService(t, root, MaxFiles)
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	err := s.GitChanges(ctx, runnerwire.GitChangesMessage{ID: "cancel", Root: root})
	if err != nil {
		if !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
		return
	}
	frame, ok := receiveFrame(t, out).(*runnerwire.GitError)
	if !ok || frame.Code != "cancelled" {
		t.Fatal(frame)
	}
}

func TestMissingRootAnswersENOENT(t *testing.T) {
	root := repositoryFixture(t)
	s, out := testService(t, root, MaxFiles)
	if err := s.GitChanges(
		t.Context(),
		runnerwire.GitChangesMessage{ID: "missing", Root: filepath.Join(root, "missing")},
	); err != nil {
		t.Fatal(err)
	}
	frame, ok := receiveFrame(t, out).(*runnerwire.GitError)
	if !ok || frame.ID != "missing" || frame.Code != "ENOENT" {
		t.Fatal(frame)
	}
}

func TestWorkingTreeRequestsWait(t *testing.T) {
	roots := make([]string, 16)
	for i := range roots {
		roots[i] = repositoryFixture(t)
		writeFixture(t, roots[i], "new", "new\n")
	}
	s, out := testService(t, roots[0], MaxFiles)
	var jobs sync.WaitGroup
	failures := make(chan error, len(roots))
	for i, root := range roots {
		jobs.Go(func() {
			failures <- s.GitChanges(t.Context(), runnerwire.GitChangesMessage{ID: fmt.Sprint(i), Root: root})
		})
	}
	jobs.Wait()
	close(failures)
	for err := range failures {
		if err != nil {
			t.Fatal(err)
		}
	}
	for range roots {
		frame := receiveFrame(t, out)
		if _, ok := frame.(*runnerwire.GitOK); !ok {
			t.Fatalf("%#v", frame)
		}
	}
}

// observeTree waits for delivered notifications, so a watched request follows
// the mutation it is testing without a wall-time sleep or a polling interval.
func observeTree(t *testing.T, root string) <-chan WatchEvent {
	t.Helper()
	events := make(chan WatchEvent, 4096)
	watch, err := StartWatch(t.Context(), []string{root}, func(event WatchEvent) { events <- event })
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := watch.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return events
}

func watchedFixture(t *testing.T, root string) (*Service, chan []byte, <-chan WatchEvent) {
	t.Helper()
	events := observeTree(t, root)
	s, out := testService(t, root, MaxFiles)
	// Native startup runs independently of the request. Each native event permits
	// another request until the service reports that startup has been adopted.
	for n := 0; ; n++ {
		if changesFixture(t, s, out, root).Watched {
			return s, out, events
		}
		writeFixture(t, root, ".host-startup", fmt.Sprint(n))
		if err := os.Remove(filepath.Join(root, ".host-startup")); err != nil {
			t.Fatal(err)
		}
		select {
		case <-events:
		case <-t.Context().Done():
			t.Fatal(t.Context().Err())
		}
	}
}

func awaitChanges(
	t *testing.T,
	s *Service,
	out <-chan []byte,
	events <-chan WatchEvent,
	root string,
	want func(runnerwire.GitChanges) bool,
) runnerwire.GitChanges {
	t.Helper()
	for {
		result := changesFixture(t, s, out, root)
		if want(result) {
			return result
		}
		// The observer and the service have independent native queues. A
		// transient file supplies another event if the observer ran first;
		// it does not invalidate Git metadata or force a whole scan.
		writeFixture(t, root, ".host-event-barrier", "barrier")
		if err := os.Remove(filepath.Join(root, ".host-event-barrier")); err != nil {
			t.Fatal(err)
		}
		select {
		case <-events:
		case <-t.Context().Done():
			t.Fatal(t.Context().Err())
		}
	}
}

func TestLaterRequestsFollowWatchAndCommit(t *testing.T) {
	root := repositoryFixture(t)
	s, out, events := watchedFixture(t, root)
	before := changesFixture(t, s, out, root)
	if len(before.Files) != 0 {
		t.Fatal(before)
	}
	writeFixture(t, root, "a.txt", "1\n2\n3\nmore\n")
	awaitChanges(t, s, out, events, root, func(c runnerwire.GitChanges) bool {
		return len(c.Files) == 1 && c.Files[0].Path == "a.txt" && c.Files[0].Kind == runnerwire.ChangeKindModified &&
			c.Files[0].Added == 1
	})
	writeFixture(t, root, "dir/new.txt", "n\n")
	if err := os.Remove(filepath.Join(root, "b.txt")); err != nil {
		t.Fatal(err)
	}
	awaitChanges(t, s, out, events, root, func(c runnerwire.GitChanges) bool {
		return len(c.Files) == 3 && c.Files[0].Path == "a.txt" && c.Files[0].Kind == runnerwire.ChangeKindModified &&
			c.Files[1].Path == "b.txt" &&
			c.Files[1].Kind == runnerwire.ChangeKindDeleted &&
			c.Files[2].Path == "dir/new.txt" &&
			c.Files[2].Kind == runnerwire.ChangeKindAdded
	})
	commitFixture(t, root)
	after := awaitChanges(t, s, out, events, root, func(c runnerwire.GitChanges) bool {
		return len(c.Files) == 0 && c.Head != nil && *c.Head != *before.Head
	})
	if !after.Watched {
		t.Fatal(after)
	}
}

func TestWatchedRequestsFollowIgnoreRulesAndModes(t *testing.T) {
	root := repositoryFixture(t)
	s, out, events := watchedFixture(t, root)
	writeFixture(t, root, "dir/new.txt", "new\n")
	writeFixture(t, root, "scratch.log", "scratch\n")
	want := gitStatuses(t, root)
	awaitChanges(
		t,
		s,
		out,
		events,
		root,
		func(c runnerwire.GitChanges) bool { return cmp.Equal(changeStatuses(c), want) },
	)
	writeFixture(t, root, ".gitignore", "*.log\n")
	want = gitStatuses(t, root)
	awaitChanges(
		t,
		s,
		out,
		events,
		root,
		func(c runnerwire.GitChanges) bool { return cmp.Equal(changeStatuses(c), want) },
	)
	if err := os.Chmod(filepath.Join(root, "b.txt"), 0o755); err != nil {
		t.Fatal(err)
	}
	want = gitStatuses(t, root)
	awaitChanges(
		t,
		s,
		out,
		events,
		root,
		func(c runnerwire.GitChanges) bool { return cmp.Equal(changeStatuses(c), want) },
	)
}

func TestRuleAboveRootReachesUnderIt(t *testing.T) {
	root := repositoryFixture(t)
	sub := filepath.Join(root, "dir")
	s, out, events := watchedFixture(t, sub)
	writeFixture(t, root, "dir/scratch.log", "scratch\n")
	awaitChanges(
		t,
		s,
		out,
		events,
		sub,
		func(c runnerwire.GitChanges) bool { return len(c.Files) == 1 && c.Files[0].Path == "scratch.log" },
	)
	writeFixture(t, root, ".gitignore", "*.log\n")
	if result := changesFixture(t, s, out, sub); len(result.Files) != 0 {
		t.Fatal(result)
	}
}

func TestStagedRenameStaysOneEntryAcrossWatch(t *testing.T) {
	root := repositoryFixture(t)
	gitCommand(t, root, "mv", "a.txt", "moved.txt")
	s, out, events := watchedFixture(t, root)
	want := runnerwire.GitChange{
		Path:   "moved.txt",
		Status: "R ",
		Kind:   runnerwire.ChangeKindRenamed,
		From:   new("a.txt"),
	}
	if diff := cmp.Diff([]runnerwire.GitChange{want}, changesFixture(t, s, out, root).Files); diff != "" {
		t.Fatal(diff)
	}
	writeFixture(t, root, "moved.txt", "1\n2\n3\n4\n")
	edited := awaitChanges(
		t,
		s,
		out,
		events,
		root,
		func(c runnerwire.GitChanges) bool { return changeMap(c)["moved.txt"].Status == "RM" },
	)
	want.Status = "RM"
	want.Added = 1
	if diff := cmp.Diff([]runnerwire.GitChange{want}, edited.Files); diff != "" {
		t.Fatal(diff)
	}
	writeFixture(t, root, "a.txt", "temporary\n")
	awaitChanges(t, s, out, events, root, func(c runnerwire.GitChanges) bool { return len(c.Files) == 2 })
	if err := os.Remove(filepath.Join(root, "a.txt")); err != nil {
		t.Fatal(err)
	}
	writeFixture(t, root, "b.txt", "changed\n")
	final := awaitChanges(t, s, out, events, root, func(c runnerwire.GitChanges) bool {
		m := changeMap(c)
		_, old := m["a.txt"]
		return !old && len(m) == 2
	})
	if diff := cmp.Diff(gitStatuses(t, root), changeStatuses(final)); diff != "" {
		t.Fatal(diff)
	}
	got := changeMap(final)["moved.txt"]
	if !reflect.DeepEqual(got.From, want.From) || got.Kind != runnerwire.ChangeKindRenamed || got.Status != "RM" ||
		got.Added != 1 ||
		got.Removed != 0 {
		t.Fatal(got)
	}
}

func TestLinkedRepositoryUsesCommonObjectsAndReferences(t *testing.T) {
	original := repositoryFixture(t)
	linked := t.TempDir()
	// Build the documented linked-worktree layout directly: the test never
	// changes the developer's worktree registry or invokes git worktree.
	dir := filepath.Join(original, ".git", "worktrees", "fixture")
	writeFixture(t, dir, "HEAD", "ref: refs/heads/main\n")
	writeFixture(t, dir, "commondir", "../..\n")
	data, err := os.ReadFile(filepath.Join(original, ".git", "index"))
	if err != nil {
		t.Fatal(err)
	}
	writeFixture(t, dir, "index", string(data))
	writeFixture(t, linked, ".git", "gitdir: "+dir+"\n")
	for path, content := range map[string]string{"a.txt": "1\n2\n3\n", "b.txt": "x\ny\n", "dir/c.txt": "c\n"} {
		writeFixture(t, linked, path, content)
	}
	writeFixture(t, linked, "a.txt", "changed\n")
	s, out := testService(t, linked, MaxFiles)
	result := changesFixture(t, s, out, linked)
	if result.Head == nil || len(result.Files) != 1 || result.Files[0].Path != "a.txt" || result.Files[0].Removed != 3 {
		t.Fatal(result)
	}
}

func TestRestoredMtimeEditAndNonTextLineCounts(t *testing.T) {
	root := repositoryFixture(t)
	info, err := os.Stat(filepath.Join(root, "a.txt"))
	if err != nil {
		t.Fatal(err)
	}
	writeFixture(t, root, "a.txt", "1\nX\n3\n")
	if err := os.Chtimes(filepath.Join(root, "a.txt"), info.ModTime(), info.ModTime()); err != nil {
		t.Fatal(err)
	}
	writeFixture(t, root, "binary", string([]byte{0, 1, 2}))
	writeFixture(t, root, "non-utf8", string([]byte{255, 254, 10}))
	s, out := testService(t, root, MaxFiles)
	changes := changeMap(changesFixture(t, s, out, root))
	if len(changes) != 3 || changes["a.txt"].Added != 1 || changes["a.txt"].Removed != 1 {
		t.Fatal(changes)
	}
	for _, name := range []string{"binary", "non-utf8"} {
		if changes[name].Added != 0 || changes[name].Removed != 0 {
			t.Fatal(changes[name])
		}
	}
}

// gitStatuses obtains Git's own complete path/status oracle for watched changes.
func gitStatuses(t *testing.T, root string) map[string]string {
	t.Helper()
	oracle := strings.Split(
		gitCommand(t, root, "--no-optional-locks", "status", "--porcelain=v1", "-z", "--untracked-files=all"),
		"\x00",
	)
	want := map[string]string{}
	for i := 0; i < len(oracle); i++ {
		item := oracle[i]
		if item == "" {
			continue
		}
		want[item[3:]] = item[:2]
		if strings.ContainsAny(item[:2], "RC") {
			i++
		}
	}
	return want
}

// changeStatuses projects the status letters reported for every changed path.
func changeStatuses(c runnerwire.GitChanges) map[string]string {
	statuses := make(map[string]string, len(c.Files))
	for _, file := range c.Files {
		statuses[file.Path] = file.Status
	}
	return statuses
}
