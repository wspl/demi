//revive:disable:unused-parameter API checkpoint retains parameter names; bodies follow after merge.

package jobs

import (
	"context"
	"io"

	"github.com/wspl/demi/internal/runnerwire"
)

// KeptOutput keeps bounded output records in read order. One job owns writes;
// readers may take snapshots concurrently. The owner must Close the writer.
type KeptOutput struct{}

// CreateKeptOutput creates the output files under directory.
func CreateKeptOutput(ctx context.Context, directory string) (*KeptOutput, error) {
	panic("not written: r-jobs")
}

// Write appends one stream read, preserving the first and newest records within
// the runner wire's kept-byte bound.
func (o *KeptOutput) Write(ctx context.Context, stream runnerwire.OutputStream, bytes []byte) error {
	panic("not written: r-jobs")
}

// Reader reads kept output while the job runs and after it ends.
func (o *KeptOutput) Reader() *KeptReader { panic("not written: r-jobs") }

// Close releases writer files without removing the output. It is idempotent.
func (o *KeptOutput) Close() error { panic("not written: r-jobs") }

// KeptReader takes snapshots of a job's kept output.
type KeptReader struct{}

// Snapshot opens the retained files at their current lengths. The caller closes
// the returned reader on success, failure or cancellation, including a partial read.
// Records are head, any left-out count, then tail; later writes do not extend it.
func (r *KeptReader) Snapshot(ctx context.Context) (io.ReadCloser, error) {
	panic("not written: r-jobs")
}
