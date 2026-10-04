package process_test

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/runner/process"
)

// The git comparison verifies the intentionally changed algorithm at the
// observable numstat boundary. Local git only; usually below 0.2 seconds.
func TestLineCountsGitNumstat(t *testing.T) {
	directory := t.TempDir()
	beforePath, afterPath := filepath.Join(directory, "before"), filepath.Join(directory, "after")
	for _, tc := range []struct {
		name, before, after string
		added, removed      uint64
	}{
		{"empty", "", "", 0, 0},
		{"added", "", "first\nlast", 2, 0},
		{"removed", "first\nlast\n", "", 0, 2},
		{"replacement", "a\nb\nc\n", "a\nx\nc\n", 1, 1},
		{"missing final newline", "same", "same\n", 1, 1},
		{"repeated lines", "a\nx\na\nx\nb\n", "a\nb\na\nx\n", 1, 2},
		{"unicode", "one\n你好\nthree\n", "你好\nthree\nfour\n", 1, 1},
		{"blank lines", "\n\na\n", "\na\n\n", 1, 1},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if err := os.WriteFile(beforePath, []byte(tc.before), 0o600); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(afterPath, []byte(tc.after), 0o600); err != nil {
				t.Fatal(err)
			}
			output, err := exec.CommandContext(
				t.Context(), "git", "-c", "diff.algorithm=myers", "diff", "--no-index",
				"--no-ext-diff", "--no-textconv", "--numstat", "--", beforePath, afterPath,
			).
				Output()
			var exit *exec.ExitError
			if err != nil && (!errors.As(err, &exit) || exit.ExitCode() != 1) {
				t.Fatalf("git: %v", err)
			}
			var gitAdded, gitRemoved uint64
			if len(output) > 0 {
				if _, err := fmt.Sscanf(string(output), "%d\t%d", &gitAdded, &gitRemoved); err != nil {
					t.Fatalf("numstat %q: %v", output, err)
				}
			}
			if gitAdded != tc.added || gitRemoved != tc.removed {
				t.Fatalf("fixture expected %d/%d, git says %d/%d", tc.added, tc.removed, gitAdded, gitRemoved)
			}
			added, removed := process.LineCounts([]byte(tc.before), []byte(tc.after))
			if added != gitAdded || removed != gitRemoved {
				t.Fatalf("counts %d/%d, git %d/%d", added, removed, gitAdded, gitRemoved)
			}
		})
	}
}

func TestLineCountsAbsentBinaryAndManyLines(t *testing.T) {
	for _, tc := range []struct {
		before, after  []byte
		added, removed uint64
	}{
		{nil, []byte("a\nb"), 2, 0},
		{[]byte("a\n"), nil, 0, 1},
		{[]byte{0xff}, []byte("text"), 0, 0},
		{nil, []byte{0}, 0, 0},
		{nil, nil, 0, 0},
	} {
		a, r := process.LineCounts(tc.before, tc.after)
		if a != tc.added || r != tc.removed {
			t.Fatalf("%x -> %x: %d/%d", tc.before, tc.after, a, r)
		}
	}
	// More than 65,535 distinct lines protects against character-sized interning.
	var before strings.Builder
	for i := 0; i < 66000; i++ {
		fmt.Fprintf(&before, "line %d\n", i)
	}
	a, r := process.LineCounts([]byte(before.String()), []byte(before.String()+"new\n"))
	if a != 1 || r != 0 {
		t.Fatalf("large line alphabet: %d/%d", a, r)
	}
}
