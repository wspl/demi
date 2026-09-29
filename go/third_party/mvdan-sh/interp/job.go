package interp

import (
	"io"
	"os"
	"sync"
)

// jobWork is shared by subshells; a task registers its children before finishing.
type jobWork struct{ tasks sync.WaitGroup }

// shellFile keeps an inherited redirection alive until every shell using it
// finishes. The native descriptor stays shared, including its seek position.
type shellFile struct {
	mu   sync.Mutex
	refs int
	file io.Closer
}

func (f *shellFile) retain() {
	f.mu.Lock()
	f.refs++
	f.mu.Unlock()
}
func (f *shellFile) Close() error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.refs--
	if f.refs == 0 {
		return f.file.Close()
	}
	return nil
}
func (r *Runner) closeFiles() {
	for file := range r.files {
		file.Close()
	}
	r.files = nil
}

// WaitBackground joins all background work, including work started by subshells.
// Call only after Run has returned, and before Reset or starting another Run.
// The context passed to Run must remain live until this method returns, or be
// canceled first to terminate the job. Foreground status is not changed.
func (r *Runner) WaitBackground() {
	r.closeFiles()
	if r.job != nil {
		r.job.tasks.Wait()
	}
}

// startTask registers background work in the job and any enclosing command
// substitution, whose captured output must be complete before expansion ends.
func (r *Runner) startTask() func() {
	r.job.tasks.Add(1)
	for _, group := range r.captureGroups {
		group.Add(1)
	}
	return func() {
		for _, group := range r.captureGroups {
			group.Done()
		}
		r.job.tasks.Done()
	}
}

// captureWriter serializes descendants writing to one expansion buffer.
type captureWriter struct {
	mu     sync.Mutex
	writer io.Writer
}

func (w *captureWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.writer.Write(p)
}

// closeUnusedFiles releases exec redirections whose last descriptor was closed
// or replaced. Subshell leases keep their own inherited descriptors alive.
func (r *Runner) closeUnusedFiles() {
	for owned := range r.files {
		file := descriptorFile(owned.file)
		if file == nil {
			continue
		}
		used := descriptorFile(r.stdin) == file || descriptorFile(r.stdout) == file || descriptorFile(r.stderr) == file
		if !used {
			for _, descriptor := range r.fds {
				if descriptorFile(descriptor.Reader) == file || descriptorFile(descriptor.Writer) == file {
					used = true
					break
				}
			}
		}
		if !used {
			owned.Close()
			delete(r.files, owned)
		}
	}
}

func descriptorFile(value any) *os.File {
	if file, ok := value.(*os.File); ok {
		return file
	}
	if wrapped, ok := value.(interface{ FileHandle() *os.File }); ok {
		return wrapped.FileHandle()
	}
	return nil
}
