//go:build linux

package procgroup_test

import (
	"bufio"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest/procgroup"
)

const helperVariable = "PROCGROUP_HELPER"

// gone reports whether the process has ended; a zombie has.
func gone(pid int) bool {
	data, err := os.ReadFile("/proc/" + strconv.Itoa(pid) + "/stat")
	if err != nil {
		return true
	}
	// The state follows the command's name in parentheses.
	fields := strings.Fields(string(data[strings.LastIndexByte(string(data), ')')+1:]))
	return len(fields) > 0 && fields[0] == "Z"
}

func untilGone(t *testing.T, pid int) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for !gone(pid) {
		if time.Now().After(deadline) {
			t.Fatalf("process %d is still there", pid)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// readPid reads the process id the child printed.
func readPid(t *testing.T, line string) int {
	t.Helper()
	pid, err := strconv.Atoi(strings.TrimSpace(line))
	if err != nil {
		t.Fatalf("the child printed %q", line)
	}
	return pid
}

// Cost: two process starts, about 0.1 s.
func TestKillEndsTheProcessAndWhatItStarted(t *testing.T) {
	t.Parallel()
	cmd := exec.Command("sh", "-c", "sleep 300 & echo $!; wait")
	out, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := procgroup.Start(cmd); err != nil {
		t.Fatal(err)
	}
	line, err := bufio.NewReader(out).ReadString('\n')
	if err != nil {
		t.Fatal(err)
	}
	grandchild := readPid(t, line)
	procgroup.Kill(cmd)
	// Wait reaps the shell; the sleep it started ended with the group.
	_ = cmd.Wait()
	untilGone(t, grandchild)
}

// TestHelperStartsAChild is the parent of the test below: it starts a child and
// waits to be killed. It does nothing in an ordinary run.
func TestHelperStartsAChild(t *testing.T) {
	if os.Getenv(helperVariable) == "" {
		t.Skip("a helper of another test")
	}
	child := exec.Command("sleep", "300")
	if err := procgroup.Start(child); err != nil {
		t.Fatal(err)
	}
	os.Stdout.WriteString(strconv.Itoa(child.Process.Pid) + "\n")
	select {}
}

// Cost: one process start, about 0.1 s.
func TestAChildEndsWithTheProcessThatStartedIt(t *testing.T) {
	t.Parallel()
	helper := exec.Command(os.Args[0], "-test.run=^TestHelperStartsAChild$")
	helper.Env = append(os.Environ(), helperVariable+"=1")
	out, err := helper.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := helper.Start(); err != nil {
		t.Fatal(err)
	}
	line, err := bufio.NewReader(out).ReadString('\n')
	if err != nil {
		t.Fatal(err)
	}
	child := readPid(t, line)
	if gone(child) {
		t.Fatal("the child ended before its parent")
	}
	// A parent that is killed cannot clean up after itself.
	_ = helper.Process.Kill()
	_ = helper.Wait()
	untilGone(t, child)
}

// Cost: two process starts, about 0.1 s.
func TestKillAdoptedEndsAnOrphanOfTheSuiteAndLeavesOthers(t *testing.T) {
	// The setting belongs to the process, and every test of the package may run
	// under it.
	if err := procgroup.AdoptOrphans(); err != nil {
		t.Skipf("no child subreaper here: %v", err)
	}
	root := t.TempDir()
	// A shell starts a job in a session of its own and exits, which orphans the
	// job; a second job belongs to no suite.
	run := func(mark string) int {
		cmd := exec.Command("sh", "-c", "setsid sleep 300 >/dev/null 2>&1 & echo $!")
		cmd.Env = append(os.Environ(), "PROCGROUP_MARK="+mark)
		out, err := cmd.Output()
		if err != nil {
			t.Fatal(err)
		}
		pid := readPid(t, string(out))
		t.Cleanup(func() {
			// This PID is our adopted child. KillAdopted may have reaped it
			// already; ECHILD then needs no cleanup.
			if err := syscall.Kill(pid, syscall.SIGKILL); err != nil && err != syscall.ESRCH {
				t.Errorf("kill adopted child: %v", err)
			}
			var status syscall.WaitStatus
			if _, err := syscall.Wait4(pid, &status, 0, nil); err != nil && err != syscall.ECHILD {
				t.Errorf("reap adopted child: %v", err)
			}
		})
		return pid
	}
	ours := run(root)
	others := run("another suite")
	procgroup.KillAdopted(root)
	if _, err := os.Stat("/proc/" + strconv.Itoa(ours)); !os.IsNotExist(err) {
		t.Fatalf("the scenario's orphan was not reaped: %v", err)
	}
	if gone(others) {
		t.Fatal("a process of another suite was ended")
	}
}

// Cost: one process start, about 0.05 s.
func TestKillAdoptedLeavesTheEndedChildOfAnotherScenarioToItsOwner(t *testing.T) {
	// A scenario's own command that ended before its Wait ran is a zombie of
	// the test process, like an orphan that ended; only its owner may reap it.
	cmd := exec.Command("true")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	untilGone(t, cmd.Process.Pid)
	procgroup.KillAdopted(t.TempDir())
	if err := cmd.Wait(); err != nil {
		t.Fatalf("its owner no longer waits for it: %v", err)
	}
}
