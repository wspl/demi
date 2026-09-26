package gnucorpus_test

import (
	"context"
	"io"
	"os"
	"os/exec"
	"testing"

	"github.com/wspl/demi/internal/testing/gnucorpus"
	"github.com/wspl/demi/internal/toolctx"
)

// upper is a stand-in utility: it echoes stdin upper-cased and, when given a
// path, also writes an "OUT" marker file next to it. It never touches real
// GNU tools, so this file protects the harness itself, not any utility.
func upper(inv *toolctx.Invocation, args []string) int {
	data, err := io.ReadAll(inv.Stdin)
	if err != nil {
		return 1
	}
	out := make([]byte, len(data))
	for i, b := range data {
		if b >= 'a' && b <= 'z' {
			b -= 'a' - 'A'
		}
		out[i] = b
	}
	if _, err := inv.Stdout.Write(out); err != nil {
		return 1
	}
	if len(args) > 1 {
		file, err := inv.Files.OpenFile(args[1], os.O_WRONLY|os.O_CREATE, 0o644)
		if err != nil {
			return 1
		}
		if _, err := file.Write([]byte("OUT")); err != nil {
			return 1
		}
		return closeOrFail(file)
	}
	return 0
}

func closeOrFail(file toolctx.File) int {
	if err := file.Close(); err != nil {
		return 1
	}
	return 0
}

func TestCheckComparesStdoutExitCodeStderrAndTree(t *testing.T) {
	cases := []gnucorpus.Case{
		{
			Name:  "plain",
			Core:  true,
			Argv:  nil,
			Stdin: []byte("ab"),
			Want:  gnucorpus.Want{Stdout: []byte("AB"), ExitCode: 0},
		},
		{
			Name:  "writes-a-file",
			Core:  true,
			Argv:  []string{"marker.txt"},
			Stdin: []byte("x"),
			Want: gnucorpus.Want{
				Stdout: []byte("X"),
				Tree:   gnucorpus.Tree{Files: map[string][]byte{"marker.txt": []byte("OUT")}},
			},
		},
	}

	gnucorpus.Check(t, "upper", upper, cases)
}

// A mismatch is reported, not silently accepted: Check must fail the test
// when the recorded expectation does not hold. This runs Check in a
// subprocess (the standard "helper process" pattern) because a subtest's
// failure always propagates up to this test regardless of what this test
// itself asserts, so the failure cannot be observed in-process without
// making this test suite itself red.
func TestCheckFailsOnMismatch(t *testing.T) {
	if os.Getenv("GNUCORPUS_HELPER_MISMATCH") == "1" {
		cases := []gnucorpus.Case{
			{Name: "wrong", Core: true, Stdin: []byte("a"), Want: gnucorpus.Want{Stdout: []byte("nope")}},
		}
		gnucorpus.Check(t, "upper", upper, cases)
		return
	}

	cmd := exec.CommandContext(context.Background(), os.Args[0], "-test.run=^TestCheckFailsOnMismatch$")
	cmd.Env = append(os.Environ(), "GNUCORPUS_HELPER_MISMATCH=1")
	output, err := cmd.CombinedOutput()
	if err == nil {
		t.Fatalf("Check did not fail on a mismatched case; helper output:\n%s", output)
	}
}
