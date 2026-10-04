package transcript

import (
	"slices"

	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
)

// PatchBatch contains one commit's patches, advancing the revision by one.
type PatchBatch struct {
	// Revision is the transcript revision after this commit.
	Revision uint64
	// Patches contains the changes in commit order.
	Patches []conversationproto.TranscriptPatch
	// Touched names added or changed blocks in change order, independent of index moves.
	Touched []types.BlockID
	// Rows names the transcript rows moved or changed.
	Rows DirtyRows
}

// DirtyRows tracks rows for the next save: an inserted suffix and changed points.
// Its zero value marks no rows.
type DirtyRows struct {
	floor  *int
	points map[int]struct{}
}

// Indices returns the rows to write out of length, in ascending order.
func (d *DirtyRows) Indices(length int) []int {
	floor := length
	if d.floor != nil {
		floor = min(*d.floor, length)
	}
	indices := make([]int, 0, len(d.points))
	for index := range d.points {
		if index < floor {
			indices = append(indices, index)
		}
	}
	slices.Sort(indices)
	for index := floor; index < length; index++ {
		indices = append(indices, index)
	}
	return indices
}

// Merge adds another commit's rows, including those a failed save did not write.
func (d *DirtyRows) Merge(other DirtyRows) {
	if other.floor != nil {
		d.moveFrom(*other.floor)
	}
	for index := range other.points {
		d.change(index)
	}
}

// moveFrom marks the transcript suffix displaced by an insertion.
func (d *DirtyRows) moveFrom(index int) {
	if d.floor == nil || index < *d.floor {
		d.floor = new(index)
	}
	for point := range d.points {
		if point >= *d.floor {
			delete(d.points, point)
		}
	}
}

// change marks one transcript row without widening an already-dirty suffix.
func (d *DirtyRows) change(index int) {
	if d.floor != nil && index >= *d.floor {
		return
	}
	if d.points == nil {
		d.points = make(map[int]struct{})
	}
	d.points[index] = struct{}{}
}

type journal struct {
	patches []conversationproto.TranscriptPatch
	touched []types.BlockID
	rows    DirtyRows
}

// touch retains transcript identities in mutation order, coalescing neighbors.
func (j *journal) touch(id types.BlockID) {
	if len(j.touched) == 0 || j.touched[len(j.touched)-1] != id {
		j.touched = append(j.touched, id)
	}
}

// add records the inserted block as it existed at insertion time.
func (j *journal) add(index int, block types.Block) {
	j.touch(block.ID())
	j.rows.moveFrom(index)
	j.patches = append(j.patches, &conversationproto.AddPatch{Index: uint32(index), Value: block})
}

// replace records a replacement without changing any prior patch's block.
func (j *journal) replace(index int, block types.Block) {
	j.touch(block.ID())
	j.rows.change(index)
	j.patches = append(j.patches, &conversationproto.ReplaceBlockPatch{Index: uint32(index), Value: block})
}

// appendText coalesces consecutive text appends to the same transcript block.
func (j *journal) appendText(index int, id types.BlockID, text string) {
	j.touch(id)
	j.rows.change(index)
	if len(j.patches) > 0 {
		if last, ok := j.patches[len(j.patches)-1].(*conversationproto.AppendTextPatch); ok &&
			last.Index == uint32(index) {
			last.Delta += text
			return
		}
	}
	j.patches = append(j.patches, &conversationproto.AppendTextPatch{Index: uint32(index), Delta: text})
}
