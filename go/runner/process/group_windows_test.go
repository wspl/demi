package process_test

import (
	"github.com/wspl/demi/go/runner/process"
	"os"
	"os/exec"
	"testing"
)

func TestMain(m *testing.M) {
	process.ExecHelper()
	os.Exit(m.Run())
}

// A native Windows worker runs this. The shell package's descendant scenario
// also exercises the same Job Object through its public Run entry point.
func TestContainedWindowsProcess(t *testing.T) {
	cmd := exec.CommandContext(t.Context(), os.Args[0], "-test.run=^$")
	group, err := process.Contain(cmd)
	if err != nil {
		t.Fatal(err)
	}
	defer group.Close()
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	if err := cmd.Wait(); err != nil {
		t.Fatal(err)
	}
	if err := group.Kill(); err != nil {
		t.Fatal(err)
	}
	if err := group.Close(); err != nil {
		t.Fatal(err)
	}
}
