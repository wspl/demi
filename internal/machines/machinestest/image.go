package machinestest

import (
	"testing"

	"github.com/wspl/demi/internal/machines/storage/storagetest"
	"github.com/wspl/demi/internal/machinewire"
)

// Entry is a Cloud archive entry supplied by storage's fixture owner.
type Entry = storagetest.Entry

// CloudImage is storage's shared fixture release, owned by its test.
type CloudImage = storagetest.CloudImage

// Entries returns the shared small Cloud root.
func Entries() []Entry { return storagetest.Entries() }

// NewCloudImage builds a release through storage's single fixture implementation.
func NewCloudImage(
	t *testing.T,
	entries []Entry,
	architecture machinewire.Architecture,
	executables ...Entry,
) *CloudImage {
	t.Helper()
	return storagetest.NewCloudImage(t, entries, architecture, executables...)
}
