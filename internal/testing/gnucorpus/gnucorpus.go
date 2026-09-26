// Package gnucorpus is a differential-corpus harness for the standard
// utilities: it runs a case's argv and stdin against a Go utility inside
// toolctxtest and compares the result byte for byte against what the GNU (or
// other reference) implementation produced when the corpus was recorded.
//
// A corpus for one utility lives at testdata/gnu/<utility>/cases.json,
// recorded by the program in internal/testing/gnucorpus/record (see
// testdata/gnu/README.md). A work package that implements a utility calls
// gnucorpus.Load to read that corpus and gnucorpus.Check to run it:
//
//	cases, err := gnucorpus.Load("cat")
//	gnucorpus.Check(t, "cat", cat.Utilities["cat"], cases)
package gnucorpus

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/internal/toolctx"
	"github.com/wspl/demi/internal/toolctx/toolctxtest"
)

// Tree is a snapshot of a directory's contents that a case cares about:
// regular files by their exact bytes, empty directories, symlinks by their
// exact target text, and the permission bits and ownership of every path.
// Inode numbers and modification times are never captured by default; they
// vary between runs and the reference tools by design. A case that sets a
// path to an explicit, deterministic time (touch -t/-d/-r, not "now") names
// it in Case.CheckTimes to have that one path's time captured and compared.
type Tree struct {
	// Files maps a path relative to the tree's root to its exact content.
	Files map[string][]byte `json:"files,omitempty"`
	// Dirs lists paths, relative to the root, that exist as empty
	// directories. A non-empty directory is implied by the paths under it
	// and is not listed here.
	Dirs []string `json:"dirs,omitempty"`
	// Symlinks maps a path relative to the root to the exact text stored as
	// its link target.
	Symlinks map[string]string `json:"symlinks,omitempty"`
	// Modes maps a path relative to the root to its permission bits
	// (os.FileMode & 0o777).
	Modes map[string]uint32 `json:"modes,omitempty"`
	// Owners maps a path relative to the root to its [uid, gid]. Recording
	// and checking are assumed to run as the same user (root in the Go
	// port's build and test containers), so a literal uid/gid recorded on
	// one host is meaningful on the other.
	Owners map[string][2]uint32 `json:"owners,omitempty"`
	// Times maps a path relative to the root to its modification time, as a
	// Unix timestamp in seconds.
	Times map[string]int64 `json:"times,omitempty"`
}

// Case is one differential-corpus case: an input to run through the
// reference implementation and, later, the Go utility, plus the reference's
// recorded output.
type Case struct {
	// Name identifies the case within its utility; it becomes the subtest
	// name, so it contains no slashes or spaces.
	Name string `json:"name"`
	// Core marks a case as exercising an option in the core set (an option
	// the old tests use, or one coding agents commonly use). false marks it
	// as extended.
	Core bool `json:"core"`
	// Argv is the utility's arguments; argv[0] (the utility's own name) is
	// added by Check and must not be included here.
	Argv []string `json:"argv"`
	// Stdin is fed to the utility.
	Stdin []byte `json:"stdin,omitempty"`
	// Tree is created under the working directory before the utility runs.
	Tree Tree `json:"tree,omitempty"`
	// Env adds to, or overrides, the fixed base environment
	// (LC_ALL=C, LANG=C, TZ=UTC, HOME=<fixed>, PATH) for this case only.
	Env map[string]string `json:"env,omitempty"`
	// CheckTimes lists paths, relative to the working directory, whose
	// resulting modification time is worth comparing. A path's time is
	// otherwise not reproducible between recording and checking, so it is
	// captured only when a case sets it to an explicit, deterministic value
	// (touch -t/-d/-r) and names it here.
	CheckTimes []string `json:"checkTimes,omitempty"`
	// Want is filled in by the recorder from the reference implementation.
	Want Want `json:"want"`
}

// Want is what a case expects: the reference implementation's recorded
// stdout, exit code, whether it wrote to stderr, and the resulting working
// directory (unchanged from Tree when the utility does not write files).
type Want struct {
	Stdout         []byte `json:"stdout"`
	ExitCode       int    `json:"exitCode"`
	StderrNonEmpty bool   `json:"stderrNonEmpty"`
	Tree           Tree   `json:"tree"`
}

// BaseEnv is the fixed environment every case runs with, before Case.Env is
// applied. home is the case's fixed HOME, inside the temp dir.
func BaseEnv(home string) map[string]string {
	return map[string]string{
		"LC_ALL": "C",
		"LANG":   "C",
		"TZ":     "UTC",
		"HOME":   home,
		// The reference recorder execs real binaries and needs PATH; the Go
		// utilities never start a process, so this is inert for them.
		"PATH": os.Getenv("PATH"),
	}
}

// MergeEnv layers extra's values over base's, without modifying either. It is
// shared by Check, which builds the Go utility's environment, and the
// recorder, which builds the reference binary's.
func MergeEnv(base map[string]string, extra map[string]string) map[string]string {
	env := make(map[string]string, len(base)+len(extra))
	for name, value := range base {
		env[name] = value
	}
	for name, value := range extra {
		env[name] = value
	}
	return env
}

// testdataDir returns testdata/gnu at the repository root, resolved from
// this source file's own location so callers in any package find the same
// directory.
func testdataDir() (string, error) {
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		return "", fs.ErrNotExist
	}
	// file is .../internal/testing/gnucorpus/gnucorpus.go; the repository
	// root is three directories up.
	root := filepath.Dir(filepath.Dir(filepath.Dir(filepath.Dir(file))))
	return filepath.Join(root, "testdata", "gnu"), nil
}

// Load reads the committed corpus for utility from
// testdata/gnu/<utility>/cases.json.
func Load(utility string) ([]Case, error) {
	dir, err := testdataDir()
	if err != nil {
		return nil, err
	}
	data, err := os.ReadFile(filepath.Join(dir, utility, "cases.json"))
	if err != nil {
		return nil, err
	}
	var cases []Case
	if err := json.Unmarshal(data, &cases); err != nil {
		return nil, err
	}
	return cases, nil
}

// CorpusPath returns testdata/gnu/<utility>/cases.json's path, for the
// recorder to write.
func CorpusPath(utility string) (string, error) {
	dir, err := testdataDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, utility, "cases.json"), nil
}

// WriteTree creates tree's files, directories and symlinks under root, which
// must already exist. It is shared by Check, which sets up a case's input
// tree, and the recorder, which sets up the same tree before running the
// reference implementation.
func WriteTree(root string, tree Tree) error {
	for _, dir := range tree.Dirs {
		if err := os.MkdirAll(filepath.Join(root, dir), 0o755); err != nil {
			return err
		}
	}
	for path, target := range tree.Symlinks {
		full := filepath.Join(root, path)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			return err
		}
		if err := os.Symlink(target, full); err != nil {
			return err
		}
	}
	// Files are created after symlinks so a file path never collides with a
	// symlink's own path; content is written before mode and ownership so
	// mode/ownership are not disturbed by the write.
	for path, content := range tree.Files {
		full := filepath.Join(root, path)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			return err
		}
		if err := os.WriteFile(full, content, 0o644); err != nil {
			return err
		}
	}
	for path, mode := range tree.Modes {
		if err := os.Chmod(filepath.Join(root, path), fs.FileMode(mode)); err != nil {
			return err
		}
	}
	for path, owner := range tree.Owners {
		if err := os.Lchown(filepath.Join(root, path), int(owner[0]), int(owner[1])); err != nil {
			return err
		}
	}
	for path, seconds := range tree.Times {
		mtime := time.Unix(seconds, 0)
		if err := os.Chtimes(filepath.Join(root, path), mtime, mtime); err != nil {
			return err
		}
	}
	return nil
}

// TimesOf reads the modification time, in Unix seconds, of each of paths
// under root. It is shared by Check and the recorder for a case's
// CheckTimes, the only paths whose time this harness compares.
func TimesOf(root string, paths []string) (map[string]int64, error) {
	if len(paths) == 0 {
		return nil, nil
	}
	times := make(map[string]int64, len(paths))
	for _, path := range paths {
		info, err := os.Lstat(filepath.Join(root, path))
		if err != nil {
			return nil, err
		}
		times[path] = info.ModTime().Unix()
	}
	return times, nil
}

// SnapshotTree walks root and returns its contents as a Tree, with every
// path relative to root and using forward slashes. It never populates
// Times; see TimesOf.
func SnapshotTree(root string) (Tree, error) {
	tree := Tree{}
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if path == root {
			return nil
		}
		rel, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		rel = filepath.ToSlash(rel)

		info, err := entry.Info()
		if err != nil {
			return err
		}
		mode := uint32(info.Mode().Perm())
		if tree.Modes == nil {
			tree.Modes = map[string]uint32{}
		}
		tree.Modes[rel] = mode
		if sys, ok := info.Sys().(*syscall.Stat_t); ok {
			if tree.Owners == nil {
				tree.Owners = map[string][2]uint32{}
			}
			tree.Owners[rel] = [2]uint32{sys.Uid, sys.Gid}
		}

		switch {
		case info.Mode()&fs.ModeSymlink != 0:
			target, err := os.Readlink(path)
			if err != nil {
				return err
			}
			if tree.Symlinks == nil {
				tree.Symlinks = map[string]string{}
			}
			tree.Symlinks[rel] = target
		case entry.IsDir():
			entries, err := os.ReadDir(path)
			if err != nil {
				return err
			}
			if len(entries) == 0 {
				tree.Dirs = append(tree.Dirs, rel)
			}
		default:
			content, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			if tree.Files == nil {
				tree.Files = map[string][]byte{}
			}
			tree.Files[rel] = content
		}
		return nil
	})
	if err != nil {
		return Tree{}, err
	}
	sort.Strings(tree.Dirs)
	return tree, nil
}

// diffTree reports the first difference between got and want, or "" when
// they match. Modes and owners are compared only for paths want records, so
// a case that does not populate them is not sensitive to them.
func diffTree(got, want Tree) string {
	for path, content := range want.Files {
		gotContent, ok := got.Files[path]
		if !ok {
			return "missing file " + path
		}
		if !bytes.Equal(gotContent, content) {
			return fmt.Sprintf("file %s: got %q, want %q", path, gotContent, content)
		}
	}
	for path := range got.Files {
		if _, ok := want.Files[path]; !ok {
			return "unexpected file " + path
		}
	}
	for _, dir := range want.Dirs {
		if !contains(got.Dirs, dir) {
			return "missing empty directory " + dir
		}
	}
	for _, dir := range got.Dirs {
		if !contains(want.Dirs, dir) {
			return "unexpected empty directory " + dir
		}
	}
	for path, target := range want.Symlinks {
		gotTarget, ok := got.Symlinks[path]
		if !ok {
			return "missing symlink " + path
		}
		if gotTarget != target {
			return "symlink " + path + ": got target " + gotTarget + ", want " + target
		}
	}
	for path := range got.Symlinks {
		if _, ok := want.Symlinks[path]; !ok {
			return "unexpected symlink " + path
		}
	}
	for path, mode := range want.Modes {
		gotMode, ok := got.Modes[path]
		if !ok {
			return "missing path for mode check " + path
		}
		if gotMode != mode {
			return fmt.Sprintf("mode of %s: got %s, want %s", path, fs.FileMode(gotMode), fs.FileMode(mode))
		}
	}
	for path, owner := range want.Owners {
		gotOwner, ok := got.Owners[path]
		if !ok {
			return "missing path for owner check " + path
		}
		if gotOwner != owner {
			return fmt.Sprintf("owner of %s: got uid=%d gid=%d, want uid=%d gid=%d",
				path, gotOwner[0], gotOwner[1], owner[0], owner[1])
		}
	}
	for path, seconds := range want.Times {
		gotSeconds, ok := got.Times[path]
		if !ok {
			return "missing path for mtime check " + path
		}
		if gotSeconds != seconds {
			return fmt.Sprintf("mtime of %s: got %d, want %d", path, gotSeconds, seconds)
		}
	}
	return ""
}

func contains(list []string, value string) bool {
	for _, item := range list {
		if item == value {
			return true
		}
	}
	return false
}

// Check runs every case in cases through utility inside toolctxtest and
// compares the result against its Want. utilityName is both the registered
// name (args[0]) and the only name Invocation.Run resolves, so a case that
// needs to run another program (find -exec, xargs) can only invoke the
// utility under test itself.
func Check(t *testing.T, utilityName string, utility toolctx.Utility, cases []Case) {
	t.Helper()
	for _, c := range cases {
		label := c.Name
		if !c.Core {
			label += "/extended"
		}
		t.Run(label, func(t *testing.T) {
			root := t.TempDir()
			work := filepath.Join(root, "work")
			home := filepath.Join(root, "home")
			for _, dir := range []string{work, home} {
				if err := os.MkdirAll(dir, 0o755); err != nil {
					t.Fatalf("create %s: %v", dir, err)
				}
			}
			if err := WriteTree(work, c.Tree); err != nil {
				t.Fatalf("set up tree: %v", err)
			}

			previousUmask := syscall.Umask(0o022)
			defer syscall.Umask(previousUmask)

			runner := &toolctxtest.Runner{
				Dir:       work,
				Env:       toolctxtest.Env(MergeEnv(BaseEnv(home), c.Env)),
				Utilities: map[string]toolctx.Utility{utilityName: utility},
			}
			args := append([]string{utilityName}, c.Argv...)
			result := runner.Run(context.Background(), utility, args, string(c.Stdin))

			if !bytes.Equal([]byte(result.Stdout), c.Want.Stdout) {
				t.Errorf("stdout = %q, want %q", result.Stdout, c.Want.Stdout)
			}
			if result.Code != c.Want.ExitCode {
				t.Errorf("exit code = %d, want %d", result.Code, c.Want.ExitCode)
			}
			if (result.Stderr != "") != c.Want.StderrNonEmpty {
				t.Errorf("stderr non-empty = %v, want %v (stderr = %q)", result.Stderr != "", c.Want.StderrNonEmpty, result.Stderr)
			}

			gotTree, err := SnapshotTree(work)
			if err != nil {
				t.Fatalf("snapshot tree: %v", err)
			}
			gotTree.Times, err = TimesOf(work, c.CheckTimes)
			if err != nil {
				t.Fatalf("read mtimes: %v", err)
			}
			if diff := diffTree(gotTree, c.Want.Tree); diff != "" {
				t.Errorf("resulting tree: %s", diff)
			}
		})
	}
}
