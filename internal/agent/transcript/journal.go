package transcript

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// PatchBatch contains one commit's patches, advancing the revision by one.
type PatchBatch struct {
	Revision uint64
	Patches  []framewire.TranscriptPatch
	// Touched names added or changed blocks in change order, independent of index moves.
	Touched []core.BlockID
	// Rows names the transcript rows moved or changed.
	Rows DirtyRows
}

// DirtyRows tracks rows for the next save: an inserted suffix and changed points.
// Its zero value marks no rows.
type DirtyRows struct{}

// Indices returns the rows to write out of length, in ascending order.
func (d *DirtyRows) Indices(length int) []int { panic("not written: a-transcript") }

// Merge adds another commit's rows, including those a failed save did not write.
func (d *DirtyRows) Merge(other DirtyRows) { panic("not written: a-transcript") }
