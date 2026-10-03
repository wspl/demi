package engine

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"mvdan.cc/sh/v3/syntax"
)

// utility runs the system program through the job's interpreter and child owner.
func utility(t *testing.T, root, name string, args ...string) (Result, string, string) {
	t.Helper()
	if _, err := exec.LookPath(name); err != nil {
		t.Skipf("decision 4: system utility %s is unavailable; provisioning is deferred: %v", name, err)
	}
	words := []string{name}
	for _, arg := range args {
		quoted, err := syntax.Quote(arg, syntax.LangBash)
		if err != nil {
			t.Fatal(err)
		}
		words = append(words, quoted)
	}
	return shellFiles(t, root, strings.Join(words, " "), nil)
}

func TestUtilitiesCwdAndExitStateArePerInvocation(t *testing.T) {
	first, second := t.TempDir(), t.TempDir()
	for root, value := range map[string]string{first: "one", second: "two"} {
		if err := os.WriteFile(filepath.Join(root, "file"), []byte(value), 0o600); err != nil {
			t.Fatal(err)
		}
		result, output, stderr := utility(t, root, "cat", "file")
		if result.Code != 0 || output != value {
			t.Fatalf("exit %d output %q stderr %q", result.Code, output, stderr)
		}
	}
	result, _, stderr := utility(t, first, "cat", "missing")
	if result.Code != 1 || !strings.HasPrefix(stderr, "cat:") {
		t.Fatalf("failure %d: %s", result.Code, stderr)
	}
	result, output, stderr := utility(t, second, "cat", "file")
	if result.Code != 0 || output != "two" {
		t.Fatalf("failure leaked: %d %q %q", result.Code, output, stderr)
	}
	t.Run("help", func(t *testing.T) {
		if runtime.GOOS == "darwin" {
			t.Skip("decision 4: BSD cat refuses --help instead of printing help to stdout")
		}
		result, output, stderr := utility(t, second, "cat", "--help")
		if result.Code != 0 || !strings.Contains(strings.ToLower(output), "usage:") {
			t.Fatalf("cat help %d %q %q", result.Code, output, stderr)
		}
	})
}

func TestEveryUtilityRoutesHelpToTheInvocationStream(t *testing.T) {
	// The reference utility registry defines the promised CLI coverage.
	for _, name := range []string{
		"cat",
		"head",
		"tail",
		"wc",
		"ls",
		"cp",
		"mv",
		"rm",
		"mkdir",
		"rmdir",
		"touch",
		"tee",
		"sort",
		"uniq",
		"cut",
		"tr",
		"paste",
		"nl",
		"tac",
		"basename",
		"dirname",
		"realpath",
		"env",
		"seq",
		"date",
		"sleep",
		"mktemp",
		"stat",
		"du",
		"df",
		"od",
		"chmod",
		"chown",
		"grep",
		"sed",
		"find",
		"xargs",
		"diff",
		"cmp",
		"jq",
		"rg",
	} {
		t.Run(name, func(t *testing.T) {
			if runtime.GOOS == "darwin" && name != "sort" && name != "tac" && name != "diff" && name != "jq" &&
				name != "rg" {
				t.Skip("decision 4: BSD utility does not print usage to stdout for --help")
			}
			result, output, stderr := utility(t, t.TempDir(), name, "--help")
			if result.Code != 0 || !strings.Contains(strings.ToLower(output), "usage:") {
				t.Fatalf("%s exit %d output %q stderr %q", name, result.Code, output, stderr)
			}
		})
	}
}

func TestFilesystemUtilitiesUseTheInvocationDirectory(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "input"), []byte("hello\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{
		{"mkdir", "-p", "nested/child"},
		{"touch", "new"},
		{"cp", "input", "copy"},
		{"mv", "copy", "moved"},
		{"ls", "-la", "."},
		{"stat", "input"},
		{"du", "input"},
		{"df", "."},
		{"rm", "moved"},
		{"rmdir", "nested/child"},
	} {
		result, output, stderr := utility(t, root, args[0], args[1:]...)
		if result.Code != 0 {
			t.Fatalf("%v: %d %s %s", args, result.Code, output, stderr)
		}
	}
	if info, err := os.Stat(filepath.Join(root, "new")); err != nil || !info.Mode().IsRegular() {
		t.Fatalf("new is not a file: %v, %v", info, err)
	}
	if _, err := os.Stat(filepath.Join(root, "moved")); !os.IsNotExist(err) {
		t.Fatalf("moved remains: %v", err)
	}
}

func TestEnvExecutesChildWithLocalEnvironmentAndStreams(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("original test is Unix-only")
	}
	root := t.TempDir()
	result, output, stderr := utility(
		t,
		root,
		"env",
		"-i",
		"DEMI_TEST_VALUE=local",
		"/bin/sh",
		"-c",
		`printf '%s' "$DEMI_TEST_VALUE"; pwd; exit 7`,
	)
	physical, err := filepath.EvalSymlinks(root)
	if err != nil {
		t.Fatal(err)
	}
	if result.Code != 7 || output != "local"+physical+"\n" {
		t.Fatalf("exit %d output %q stderr %q", result.Code, output, stderr)
	}
	if _, ok := os.LookupEnv("DEMI_TEST_VALUE"); ok {
		t.Fatal("child environment escaped")
	}
}

func TestRecursiveOperationsStayInTheInvocationDirectory(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "source/child"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "source/child/file"), []byte("nested"), 0o600); err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{
		{"cp", "-R", "source", "copy"},
		{"du", "copy"},
		{"ls", "-R", "copy"},
		{"rm", "-r", "copy"},
	} {
		result, output, stderr := utility(t, root, args[0], args[1:]...)
		if result.Code != 0 {
			t.Fatalf("%v: %d %s %s", args, result.Code, output, stderr)
		}
		if args[0] == "cp" {
			data, err := os.ReadFile(filepath.Join(root, "copy/child/file"))
			if err != nil || string(data) != "nested" {
				t.Fatalf("copy %q: %v", data, err)
			}
		}
	}
	if _, err := os.Stat(filepath.Join(root, "copy")); !os.IsNotExist(err) {
		t.Fatalf("copy remains: %v", err)
	}
	if _, err := os.Stat(filepath.Join(root, "source/child/file")); err != nil {
		t.Fatal(err)
	}
}

func TestConcurrentExternalSortsOwnTheirTemporaryFilesAndWorkers(t *testing.T) {
	for index := range 2 {
		t.Run(fmt.Sprint(index), func(t *testing.T) {
			t.Parallel()
			root := t.TempDir()
			if err := os.Mkdir(filepath.Join(root, "temporary"), 0o700); err != nil {
				t.Fatal(err)
			}
			var text, want strings.Builder
			for number := 1999; number >= 0; number-- {
				fmt.Fprintf(&text, "%04d-%d\n", number, index)
			}
			for number := range 2000 {
				fmt.Fprintf(&want, "%04d-%d\n", number, index)
			}
			if err := os.WriteFile(filepath.Join(root, "input"), []byte(text.String()), 0o600); err != nil {
				t.Fatal(err)
			}
			args := []string{"--parallel=2", "-S", "1K", "-T", "temporary", "input"}
			result, output, stderr := utility(t, root, "sort", args...)
			if result.Code != 0 || output != want.String() {
				t.Fatalf("exit %d bytes %d stderr %q", result.Code, len(output), stderr)
			}
			entries, err := os.ReadDir(filepath.Join(root, "temporary"))
			if err != nil || len(entries) != 0 {
				t.Fatalf("temporary files %v: %v", entries, err)
			}
		})
	}
}

func TestRelativeSymlinksAndExplicitDirectoryModesArePreserved(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("original test is Unix-only")
	}
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "target"), []byte("bytes"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("target", filepath.Join(root, "link")); err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{
		{"cp", "-P", "link", "copy"},
		{"touch", "-h", "link"},
		{"mkdir", "-m", "0777", "directory"},
	} {
		result, output, stderr := utility(t, root, args[0], args[1:]...)
		if result.Code != 0 {
			t.Fatalf("%v: %d %q %q", args, result.Code, output, stderr)
		}
	}
	link, err := os.Readlink(filepath.Join(root, "copy"))
	if err != nil || link != "target" {
		t.Fatalf("link %q: %v", link, err)
	}
	info, err := os.Stat(filepath.Join(root, "directory"))
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o777 {
		t.Fatalf("directory mode %o", info.Mode().Perm())
	}
}

func TestSearchEditAndCompareUtilitiesKeepTheirCLIAndLocalPaths(t *testing.T) {
	if runtime.GOOS == "darwin" {
		t.Skip("decision 4: BSD sed requires an extension after -i")
	}
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "tree"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "tree/input"), []byte("apple\npear\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	result, output, stderr := utility(t, root, "grep", "-rn", "apple", "tree")
	if result.Code != 0 || output != "tree/input:1:apple\n" {
		t.Fatalf("grep %d %q %q", result.Code, output, stderr)
	}
	result, output, stderr = utility(t, root, "find", "tree", "-type", "f", "-name", "input")
	if result.Code != 0 || output != "tree/input\n" {
		t.Fatalf("find %d %q %q", result.Code, output, stderr)
	}
	args := []string{"-i", "s/apple/orange/", "tree/input"}

	result, output, stderr = utility(t, root, "sed", args...)
	if result.Code != 0 {
		t.Fatalf("sed %d %q %q", result.Code, output, stderr)
	}
	data, err := os.ReadFile(filepath.Join(root, "tree/input"))
	if err != nil || string(data) != "orange\npear\n" {
		t.Fatalf("edited %q: %v", data, err)
	}
	if err := os.WriteFile(filepath.Join(root, "other"), []byte("different\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"diff", "cmp"} {
		result, _, stderr = utility(t, root, name, "tree/input", "other")
		if result.Code != 1 {
			t.Fatalf("%s comparison %d: %s", name, result.Code, stderr)
		}
		result, _, stderr = utility(t, root, name, "tree/input", "tree/input")
		if result.Code != 0 {
			t.Fatalf("%s equality %d: %s", name, result.Code, stderr)
		}
	}
}

func TestJqUsesFullFiltersFilesArgumentsAndInvocationEnvironment(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "input.json"), []byte(`[{"x":1},{"x":3}]`), 0o600); err != nil {
		t.Fatal(err)
	}
	result, output, stderr := utility(
		t,
		root,
		"jq",
		"-c",
		"--argjson",
		"min",
		"2",
		"map(select(.x > $min)) | .[].x",
		"input.json",
	)
	if result.Code != 0 || output != "3\n" {
		t.Fatalf("jq %d %q %q", result.Code, output, stderr)
	}
	jq, err := exec.LookPath("jq")
	if err != nil {
		t.Fatal(err)
	}
	result, output, stderr = utility(t, root, "env", "-i", jq, "-n", "-c", "env == $ENV and (env | length == 0)")
	if result.Code != 0 || output != "true\n" {
		t.Fatalf("jq environment %d %q %q", result.Code, output, stderr)
	}
	result, _, _ = utility(t, root, "jq", "-n", "-e", "false")
	if result.Code != 1 {
		t.Fatalf("false: %d", result.Code)
	}
	result, _, _ = utility(t, root, "jq", "-n", "bad_syntax(")
	if result.Code != 3 {
		t.Fatalf("syntax: %d", result.Code)
	}
}

func TestRipgrepSearchesAndFiltersLocalFilesWithUpstreamOptions(t *testing.T) {
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "tree"), 0o700); err != nil {
		t.Fatal(err)
	}
	for name, data := range map[string]string{
		".gitignore": "ignored\n",
		"ignored":    "apple\n",
		"input":      "apple\npear\nAPPLE\n",
	} {
		if err := os.WriteFile(filepath.Join(root, "tree", name), []byte(data), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	result, output, stderr := utility(t, root, "rg", "-j2", "--no-require-git", "-ni", "apple", "tree")
	if result.Code != 0 || output != "tree/input:1:apple\ntree/input:3:APPLE\n" {
		t.Fatalf("rg %d %q %q", result.Code, output, stderr)
	}
	result, output, stderr = utility(t, root, "rg", "--json", "pear", "tree/input")
	if result.Code != 0 || !strings.Contains(output, `"type":"match"`) {
		t.Fatalf("rg JSON %d %q %q", result.Code, output, stderr)
	}
	result, _, _ = utility(t, root, "rg", "not-present", "tree")
	if result.Code != 1 {
		t.Fatalf("missing: %d", result.Code)
	}
	result, _, _ = utility(t, root, "rg", "[", "tree")
	if result.Code != 2 {
		t.Fatalf("syntax: %d", result.Code)
	}
}
