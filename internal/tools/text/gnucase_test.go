package text_test

import (
	"context"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/toolctx/toolctxtest"
	"github.com/wspl/demi/internal/tools/text"
)

// gnuCase is one run of a utility whose expected result was recorded from
// GNU coreutils 9.4 with LC_ALL=C.
type gnuCase struct {
	Name string
	// Args starts with the utility's name.
	Args  []string
	Stdin string
	// Files is the tree created in an empty working directory first. A path
	// ending in "/" is a directory.
	Files  map[string]string
	Stdout string
	Code   int
	Stderr string
	// After is the tree after the run when the utility changed it.
	After map[string]string
}

// checkCases runs every case in a fresh directory and compares standard
// output, exit status, standard error and the resulting tree.
func checkCases(t *testing.T, cases []gnuCase) {
	t.Helper()
	for _, c := range cases {
		t.Run(c.Name, func(t *testing.T) {
			dir := t.TempDir()
			writeTree(t, dir, c.Files)
			runner := &toolctxtest.Runner{
				Dir: dir,
				Env: toolctxtest.Env{"LC_ALL": "C", "TZ": "UTC", "HOME": dir},
			}
			utility, ok := text.Utilities[c.Args[0]]
			if !ok {
				t.Fatalf("no utility %q", c.Args[0])
			}

			result := runner.Run(context.Background(), utility, c.Args, c.Stdin)

			if result.Stdout != c.Stdout {
				t.Errorf("stdout = %q, want %q", result.Stdout, c.Stdout)
			}
			if result.Code != c.Code {
				t.Errorf("exit status = %d, want %d", result.Code, c.Code)
			}
			if result.Stderr != c.Stderr {
				t.Errorf("stderr = %q, want %q", result.Stderr, c.Stderr)
			}
			want := c.After
			if want == nil {
				want = c.Files
			}
			got := readTree(t, dir)
			if !sameTree(got, want) {
				t.Errorf("tree = %q, want %q", got, want)
			}
		})
	}
}

func writeTree(t *testing.T, dir string, files map[string]string) {
	t.Helper()
	for name, content := range files {
		path := filepath.Join(dir, name)
		if strings.HasSuffix(name, "/") {
			if err := os.MkdirAll(path, 0o755); err != nil {
				t.Fatal(err)
			}
			continue
		}
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
}

func readTree(t *testing.T, dir string) map[string]string {
	t.Helper()
	tree := map[string]string{}
	err := filepath.WalkDir(dir, func(path string, entry fs.DirEntry, err error) error {
		if err != nil || path == dir {
			return err
		}
		name, err := filepath.Rel(dir, path)
		if err != nil {
			return err
		}
		if entry.IsDir() {
			tree[name+"/"] = ""
			return nil
		}
		data, err := os.ReadFile(path)
		tree[name] = string(data)
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	return tree
}

// sameTree compares trees, counting a directory as present when a file
// inside it is listed.
func sameTree(got, want map[string]string) bool {
	complete := map[string]string{}
	for name, content := range want {
		complete[name] = content
		for dir := filepath.Dir(strings.TrimSuffix(name, "/")); dir != "."; dir = filepath.Dir(dir) {
			complete[dir+"/"] = ""
		}
	}
	if len(got) != len(complete) {
		return false
	}
	for name, content := range complete {
		if other, ok := got[name]; !ok || other != content {
			return false
		}
	}
	return true
}
