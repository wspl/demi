//revive:disable:unused-parameter API checkpoint retains parameter names; bodies follow after merge.

package jobs

import "context"

// Directories owns job directories under one installation's job root for a
// connection. Its methods are safe for concurrent use.
type Directories struct{}

// OpenDirectories removes abandoned directories under root. Cleanup failures
// are logged, as on connection shutdown; they do not refuse the connection.
func OpenDirectories(ctx context.Context, root string) *Directories { panic("not written: r-jobs") }

// Root returns the installation's job root, where the edit lock lives too.
func (d *Directories) Root() string { panic("not written: r-jobs") }

// Create makes a private directory, scratch area and kept output for job.
// The caller must Finish the returned directory on every exit path.
func (d *Directories) Create(ctx context.Context, job string) (*Directory, error) {
	panic("not written: r-jobs")
}

// Output returns a reader while job's directory lasts.
func (d *Directories) Output(job string) (*KeptReader, bool) { panic("not written: r-jobs") }

// Release removes an ended job's directory. Running jobs stay; removal failures
// are logged without changing the job result.
func (d *Directories) Release(ctx context.Context, job string) { panic("not written: r-jobs") }

// Clear removes every directory except those of running jobs. The connection
// calls it after joining jobs, and OpenDirectories calls it at startup.
func (d *Directories) Clear(ctx context.Context) { panic("not written: r-jobs") }

// Directory holds a job's kept output and scratch directory until Finish.
// Finish preserves Path and Output's readable data until Directories.Release.
type Directory struct {
	Path    string
	Scratch string
	Output  *KeptOutput
}

// Finish closes the output writer, removes scratch and releases the running
// hold, even when cleanup fails. Its owner first joins all job IO. It is idempotent.
func (d *Directory) Finish(ctx context.Context) error { panic("not written: r-jobs") }
