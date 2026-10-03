package jobs

import (
	"context"
	"errors"
	"log/slog"
	"os"
	"path/filepath"
	"sync"
)

// Directories owns job directories under one installation's job root for a
// connection. Its methods are safe for concurrent use.
type Directories struct {
	root string
	// mu protects job ownership; filesystem work occurs after unlocking.
	mu   sync.Mutex
	jobs map[string]*Directory
}

// OpenDirectories removes abandoned directories under root. Cleanup failures
// are logged, as on connection shutdown; they do not refuse the connection.
func OpenDirectories(ctx context.Context, root string) *Directories {
	d := &Directories{root: root, jobs: make(map[string]*Directory)}
	d.Clear(ctx)
	return d
}

// Root returns the installation's job root, where the edit lock lives too.
func (d *Directories) Root() string {
	return d.root
}

// Create makes a private directory, scratch area and kept output for job.
// The caller must Finish the returned directory on every exit path.
func (d *Directories) Create(ctx context.Context, job string) (*Directory, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if err := os.MkdirAll(d.root, 0o700); err != nil {
		return nil, err
	}
	path, err := os.MkdirTemp(d.root, "job-")
	if err != nil {
		return nil, err
	}
	success := false
	defer func() {
		if !success {
			_ = os.RemoveAll(path)
		}
	}() // Failed construction leaves no directory.
	scratch, err := os.MkdirTemp(path, ".work-")
	if err != nil {
		return nil, err
	}
	output, err := CreateKeptOutput(ctx, filepath.Join(path, "output"))
	if err != nil {
		return nil, err
	}
	directory := &Directory{Path: path, Scratch: scratch, Output: output, owner: d, job: job, running: true}
	d.mu.Lock()
	d.jobs[job] = directory
	d.mu.Unlock()
	success = true
	return directory, nil
}

// Output returns a reader while job's directory lasts.
func (d *Directories) Output(job string) (*KeptReader, bool) {
	d.mu.Lock()
	directory := d.jobs[job]
	d.mu.Unlock()
	if directory == nil {
		return nil, false
	}
	return directory.Output.Reader(), true
}

// Release removes an ended job's directory. Running jobs stay; removal failures
// are logged without changing the job result.
func (d *Directories) Release(ctx context.Context, job string) {
	d.mu.Lock()
	directory := d.jobs[job]
	if directory != nil && !directory.running {
		delete(d.jobs, job)
	} else {
		directory = nil
	}
	d.mu.Unlock()
	if directory != nil {
		removeDirectory(ctx, directory.Path)
	}
}

// Clear removes every directory except those of running jobs. The connection
// calls it after joining jobs, and OpenDirectories calls it at startup.
func (d *Directories) Clear(ctx context.Context) {
	d.mu.Lock()
	running := make(map[string]bool)
	for job, directory := range d.jobs {
		if directory.running {
			running[directory.Path] = true
		} else {
			delete(d.jobs, job)
		}
	}
	d.mu.Unlock()
	entries, err := os.ReadDir(d.root)
	if errors.Is(err, os.ErrNotExist) {
		return
	}
	if err != nil {
		slog.Warn("the job directories could not be listed", "error", err)
		return
	}
	for _, entry := range entries {
		path := filepath.Join(d.root, entry.Name())
		if entry.IsDir() && !running[path] {
			removeDirectory(ctx, path)
		}
	}
}

// Directory holds a job's kept output and scratch directory until Finish.
// Finish preserves Path and Output's readable data until Directories.Release.
type Directory struct {
	// Path is the retained job directory.
	Path string
	// Scratch is the temporary job workspace removed by Finish.
	Scratch string
	// Output owns the retained job output.
	Output  *KeptOutput
	owner   *Directories
	job     string
	running bool
	once    sync.Once
	err     error
}

// Finish closes the output writer, removes scratch and releases the running
// hold, even when cleanup fails. Its owner first joins all job IO. It is idempotent.
func (d *Directory) Finish(_ context.Context) error {
	d.once.Do(func() {
		d.err = errors.Join(d.Output.Close(), os.RemoveAll(d.Scratch))
		d.owner.mu.Lock()
		d.running = false
		d.owner.mu.Unlock()
	})
	return d.err
}

// removeDirectory removes retained job data without turning cleanup into job failure.
func removeDirectory(ctx context.Context, path string) {
	if err := ctx.Err(); err != nil {
		return
	}
	if err := os.RemoveAll(path); err != nil {
		slog.Warn("a job directory was not removed", "directory", path, "error", err)
	}
}
