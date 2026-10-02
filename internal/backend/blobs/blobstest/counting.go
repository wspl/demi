// Named parameters document this API checkpoint; bodies follow after its merge.
//revive:disable:unused-parameter

package blobstest

import "github.com/wspl/demi/internal/backend/blobs"

// ObjectCounts counts the operations reaching one object store. Its zero value
// is ready to use; share its pointer across observers rather than copying it.
// It supports concurrent observation and snapshots.
type ObjectCounts struct{}

// ObjectTally is what reached the object store up to one moment.
type ObjectTally struct {
	Puts uint64
	// BytesPut is the bytes the puts sent.
	BytesPut uint64
	// Gets counts reads of an object's bytes.
	Gets uint64
	// Heads counts asks whether an object exists, which transfer no bytes.
	Heads uint64
	// MostGetsAtOnce is the most reads that were in flight at once.
	MostGetsAtOnce uint64
	// Lists counts listings of a prefix.
	Lists uint64
	// Deletes counts objects asked to be deleted.
	Deletes uint64
}

// Since returns what reached the store since earlier; the most reads at once
// is this tally's.
func (t ObjectTally) Since(earlier ObjectTally) ObjectTally {
	panic("not written: b-blobs")
}

// Tally returns a snapshot of the operations observed so far.
func (c *ObjectCounts) Tally() ObjectTally {
	panic("not written: b-blobs")
}

// Observe returns objects with what reaches it counted here. It borrows objects;
// the caller retains ownership of the underlying bucket and its cleanup.
func (c *ObjectCounts) Observe(objects blobs.Objects) blobs.Objects {
	panic("not written: b-blobs")
}
