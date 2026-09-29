//go:build linux

package procgroup

import (
	"bytes"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

// A start is a request to the launcher goroutine.
type start struct {
	cmd  *exec.Cmd
	done chan error
}

// started are the processes Start started: their owners wait for them.
var started struct {
	mu   sync.Mutex
	pids map[int]bool
}

var launcher struct {
	once     sync.Once
	requests chan start
}

// launch runs every start of the process on one goroutine locked to its OS
// thread. The kernel sends a parent-death signal when the thread that started
// a child ends, not when the process does, so the children must come from a
// thread that lives as long as the process. The goroutine never unlocks and
// never returns: it ends with the process, and a goroutine that cannot be
// stopped is documented at its start.
func launch() {
	runtime.LockOSThread()
	for request := range launcher.requests {
		request.done <- request.cmd.Start()
	}
}

// Start starts cmd in a process group of its own, with SIGKILL for it when the
// launcher thread ends, which is when this process does.
func Start(cmd *exec.Cmd) error {
	launcher.once.Do(func() {
		launcher.requests = make(chan start)
		go launch()
	})
	if cmd.SysProcAttr == nil {
		cmd.SysProcAttr = &syscall.SysProcAttr{}
	}
	cmd.SysProcAttr.Setpgid = true
	cmd.SysProcAttr.Pdeathsig = syscall.SIGKILL
	done := make(chan error, 1)
	launcher.requests <- start{cmd: cmd, done: done}
	err := <-done
	if err == nil {
		started.mu.Lock()
		if started.pids == nil {
			started.pids = map[int]bool{}
		}
		started.pids[cmd.Process.Pid] = true
		started.mu.Unlock()
	}
	return err
}

// prSetChildSubreaper is PR_SET_CHILD_SUBREAPER of prctl(2).
const prSetChildSubreaper = 36

// AdoptOrphans makes this process the parent of every process that loses its
// own: a runner killed in the middle of a job leaves the job's processes to
// it, which KillAdopted then ends. The setting belongs to the whole process.
func AdoptOrphans() error {
	if _, _, errno := syscall.RawSyscall(syscall.SYS_PRCTL, prSetChildSubreaper, 1, 0); errno != 0 {
		return errno
	}
	return nil
}

// KillAdopted ends the processes this process adopted that belong to a suite
// whose files are under root, that is, whose environment or directory names
// it, and waits for each it killed. It touches nothing else: a process that
// Start started, or that a scenario started plainly, is its starter's to wait
// for, and an ended process shows nothing of the suite it belonged to.
func KillAdopted(root string) {
	self := os.Getpid()
	deadline := time.Now().Add(10 * time.Second)
	// What an adopted process left behind is adopted in its turn, so the scan
	// goes on until none is left.
	for time.Now().Before(deadline) {
		found := false
		entries, err := os.ReadDir("/proc")
		if err != nil {
			return
		}
		for _, entry := range entries {
			pid, err := strconv.Atoi(entry.Name())
			if err != nil {
				continue
			}
			state, parent, ok := stat(pid)
			if !ok || parent != self || isStarted(pid) {
				continue
			}
			if state == 'Z' || !belongsTo(pid, root) {
				// Reaping an ended process here could take its exit status
				// from the scenario that waits for it.
				continue
			}
			found = true
			// The process and its group, which the orphan may lead; both may
			// be gone already.
			_ = syscall.Kill(-pid, syscall.SIGKILL)
			_ = syscall.Kill(pid, syscall.SIGKILL)
			var status syscall.WaitStatus
			_, _ = syscall.Wait4(pid, &status, 0, nil)
		}
		if !found {
			return
		}
	}
}

func isStarted(pid int) bool {
	started.mu.Lock()
	defer started.mu.Unlock()
	return started.pids[pid]
}

// stat reads a process's state and parent from /proc; ok is false for a
// process that is gone.
func stat(pid int) (state byte, parent int, ok bool) {
	data, err := os.ReadFile("/proc/" + strconv.Itoa(pid) + "/stat")
	if err != nil {
		return 0, 0, false
	}
	// The state and the parent follow the command's name in parentheses.
	rest := strings.Fields(string(data[bytes.LastIndexByte(data, ')')+1:]))
	if len(rest) < 2 {
		return 0, 0, false
	}
	parent, err = strconv.Atoi(rest[1])
	if err != nil {
		return 0, 0, false
	}
	return rest[0][0], parent, true
}

// belongsTo reports whether the process's environment or working directory
// names root.
func belongsTo(pid int, root string) bool {
	prefix := "/proc/" + strconv.Itoa(pid)
	if environment, err := os.ReadFile(prefix + "/environ"); err == nil && bytes.Contains(environment, []byte(root)) {
		return true
	}
	if directory, err := os.Readlink(prefix + "/cwd"); err == nil && strings.HasPrefix(directory, root) {
		return true
	}
	return false
}
