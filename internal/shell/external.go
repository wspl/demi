package shell

import (
	"context"
	"errors"
	"io"
	"os"
	"reflect"
	"sync"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

// runExternal runs the program at path in a process group of its own and
// returns its exit status once the program has exited, the rest of its group
// has been killed, and the output it wrote through the job has been copied.
// Cancelling ctx kills the group.
func (j *Job) runExternal(ctx context.Context, path string, cmd command) (int, error) {
	child := childStreams{job: j, ctx: ctx}
	defer child.closeChildEnds()
	files, err := child.files(cmd.streams)
	if err != nil {
		return 126, err
	}
	env := []string(cmd.env)
	if env == nil {
		// A nil environment would inherit the runner's.
		env = []string{}
	}
	attr := &os.ProcAttr{
		Dir:   cmd.dir,
		Env:   env,
		Files: files,
		Sys:   &syscall.SysProcAttr{Setpgid: true},
	}
	process, err := j.startProcess(ctx, path, cmd.args, attr)
	child.closeChildEnds()
	if err != nil {
		child.waitOutputs()
		return 126, err
	}
	code, err := reap(ctx, process)
	if copyErr := child.waitOutputs(); err == nil && copyErr != nil {
		// Output forwarded into a file failed to reach it.
		return 1, copyErr
	}
	return code, err
}

// startProcess starts a program, waiting for descriptors when none is left.
func (j *Job) startProcess(ctx context.Context, path string, args []string, attr *os.ProcAttr) (*os.Process, error) {
	return waitForDescriptors(ctx, func() (*os.Process, error) {
		process, err := os.StartProcess(path, args, attr)
		// Another process of the runner that forks at the same time may
		// briefly hold a descriptor that writes the program, which makes
		// the exec fail with ETXTBSY (https://go.dev/issue/22315). The window
		// is short, so try again.
		for pause := time.Millisecond; errors.Is(err, syscall.ETXTBSY) && pause < 300*time.Millisecond; pause *= 2 {
			time.Sleep(pause)
			process, err = os.StartProcess(path, args, attr)
		}
		return process, err
	})
}

// reap waits for a program to exit, kills what remains of its process group,
// and returns its exit status: its code, or 128 plus the signal that ended
// it. Cancelling ctx kills the group at once.
func reap(ctx context.Context, process *os.Process) (int, error) {
	var mu sync.Mutex
	reaped := false
	kill := func() {
		mu.Lock()
		defer mu.Unlock()
		if !reaped {
			killGroup(process.Pid)
		}
	}
	stop := context.AfterFunc(ctx, kill)
	defer stop()
	// Wait without reaping: until the leader is reaped, its process group
	// ID cannot name another group.
	if err := waitExited(process.Pid); err != nil {
		kill()
	}
	// Job-owned work never outlives its command: descendants still in the
	// group are killed with it.
	kill()
	mu.Lock()
	reaped = true
	state, err := process.Wait()
	mu.Unlock()
	if err != nil {
		return 1, err
	}
	status, ok := state.Sys().(syscall.WaitStatus)
	if ok && status.Signaled() {
		return 128 + int(status.Signal()), nil
	}
	return state.ExitCode(), nil
}

// waitExited waits until the process has exited, without reaping it.
func waitExited(pid int) error {
	for {
		var info unix.Siginfo
		err := unix.Waitid(unix.P_PID, pid, &info, unix.WEXITED|unix.WNOWAIT, nil)
		if !errors.Is(err, syscall.EINTR) {
			return err
		}
	}
}

// killGroup kills the process group led by pid.
func killGroup(pid int) {
	// ESRCH means that no process is left in the group, which is the goal.
	syscall.Kill(-pid, syscall.SIGKILL)
}

// childStreams gives a program the files for its descriptors. A stream that
// is not a file is connected through an OS pipe and a copy.
type childStreams struct {
	job *Job
	ctx context.Context
	// childEnds are the pipe ends only the program uses; the job closes its
	// copies once the program has started.
	childEnds []*os.File
	// outputs counts the copies of the program's output into the job.
	outputs sync.WaitGroup
	mu      sync.Mutex
	// outputErr is the first error of an output copy.
	outputErr error
}

// files returns the program's descriptors: standard input, output and error,
// and those above 2 that are files or pipes.
func (c *childStreams) files(s streams) ([]*os.File, error) {
	stdin, err := c.input(s.stdin)
	if err != nil {
		return nil, err
	}
	stdout, err := c.output(s.stdout)
	if err != nil {
		return nil, err
	}
	stderr := stdout
	if !sameStream(unguardedWriter(s.stdout), unguardedWriter(s.stderr)) {
		stderr, err = c.output(s.stderr)
		if err != nil {
			return nil, err
		}
	}
	files := []*os.File{stdin, stdout, stderr}
	for fd, d := range s.descriptors {
		if fd < 3 {
			continue
		}
		file, err := c.inherited(d.Reader, d.Writer)
		if err != nil {
			return nil, err
		}
		if file == nil {
			continue
		}
		for len(files) <= fd {
			files = append(files, nil)
		}
		files[fd] = file
	}
	return files, nil
}

// sameStream reports whether a and b are the same stream, as after 2>&1.
func sameStream(a, b any) bool {
	if a == nil || reflect.TypeOf(a) != reflect.TypeOf(b) || !reflect.TypeOf(a).Comparable() {
		return false
	}
	return a == b
}

// input returns the file a program reads as its standard input.
func (c *childStreams) input(stream io.Reader) (*os.File, error) {
	switch stream := unguardedReader(stream).(type) {
	case nil:
		return nil, nil // closed, as by "<&-"
	case *os.File:
		return stream, nil
	case *pipeReader:
		return stream.file(c.ctx)
	case *jobFile:
		return stream.file, nil
	default:
		r, w, err := osPipe(c.ctx)
		if err != nil {
			return nil, err
		}
		c.childEnds = append(c.childEnds, r)
		c.job.goTask(func() {
			// The copy stops at the end of the input, or once the program
			// is gone and writing fails; either way there is nothing to report.
			io.Copy(w, stream)
			w.Close()
		})
		return r, nil
	}
}

// output returns the file a program writes as its standard output or error.
func (c *childStreams) output(stream io.Writer) (*os.File, error) {
	switch stream := unguardedWriter(stream).(type) {
	case nil:
		return nil, nil
	case *os.File:
		return stream, nil
	case *pipeWriter:
		return stream.file(c.ctx)
	case *jobFile:
		if stream.recorder == nil {
			return stream.file, nil
		}
		// Output redirected into a recorded file reaches it through the
		// recorder (docs/demi-next/edit-tracking.md).
		return c.forward(stream)
	default:
		if stream == io.Discard {
			return nil, nil // closed, as by ">&-"
		}
		return c.forward(stream)
	}
}

// forward connects a program's output to stream through an OS pipe, and
// copies until every holder of the pipe has closed it.
func (c *childStreams) forward(stream io.Writer) (*os.File, error) {
	r, w, err := osPipe(c.ctx)
	if err != nil {
		return nil, err
	}
	c.childEnds = append(c.childEnds, w)
	c.outputs.Add(1)
	c.job.goTask(func() {
		defer c.outputs.Done()
		// writerOnly keeps io.Copy from handing the pipe to a ReadFrom that
		// would bypass the recorder.
		_, err := io.Copy(writerOnly{stream}, r)
		// The pipe is drained or failed; its read end has no other user.
		r.Close()
		if err != nil {
			c.mu.Lock()
			if c.outputErr == nil {
				c.outputErr = err
			}
			c.mu.Unlock()
		}
	})
	return w, nil
}

// inherited returns the file behind a descriptor above 2 that a program
// inherits, or nil for a stream that is not a file or pipe. Writes a program
// makes through such a descriptor are not recorded.
func (c *childStreams) inherited(reader io.Reader, writer io.Writer) (*os.File, error) {
	for _, end := range [...]any{writer, reader} {
		switch end := end.(type) {
		case *os.File:
			return end, nil
		case *jobFile:
			return end.file, nil
		case *pipeWriter:
			return end.file(c.ctx)
		case *pipeReader:
			return end.file(c.ctx)
		}
	}
	return nil, nil
}

// closeChildEnds closes the job's copies of the pipe ends only the program
// uses; calling it again does nothing.
func (c *childStreams) closeChildEnds() {
	for _, file := range c.childEnds {
		// The program has its own copy; ours has no other user.
		file.Close()
	}
	c.childEnds = nil
}

// waitOutputs waits for the output copies and returns the first error.
func (c *childStreams) waitOutputs() error {
	c.outputs.Wait()
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.outputErr
}

// writerOnly hides every method of a writer but Write.
type writerOnly struct{ w io.Writer }

func (w writerOnly) Write(b []byte) (int, error) { return w.w.Write(b) }
