// Named parameters document this API checkpoint; bodies follow after its merge.
//revive:disable:unused-parameter

package blobs

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Stores holds every user's blob namespace, with the backend's record of blob uses.
// Its namespaces share the use record and support concurrent calls.
type Stores struct{}

// New returns the namespaces in objects, whose uses are timed by clock.
// It borrows objects; its caller retains responsibility for closing the bucket.
func New(objects Objects, clock core.Clock) *Stores {
	panic("not written: b-blobs")
}

// ForUser returns user's namespace: the signed-in user's for uploads and
// downloads, the conversation owner's for transcript media.
func (s *Stores) ForUser(user webapi.UserID) *Namespace {
	panic("not written: b-blobs")
}

// Namespace holds one user's blobs and implements the session's store.BlobStore.
// It borrows its store and supports concurrent calls.
type Namespace struct{}

var _ store.BlobStore = (*Namespace)(nil)

// Stored is a blob of a namespace, as its listing finds it.
type Stored struct {
	Blob core.BlobRef
	// Written is when its object was written.
	Written core.Timestamp
}

// Put stores data and answers its SHA-256 name. A name already present is
// success without sending bytes. It records a use before checking existence;
// a put that meets a deletion waits for it to end and then stores the bytes again.
func (n *Namespace) Put(ctx context.Context, data core.B64Bytes) (core.BlobRef, error) {
	panic("not written: b-blobs")
}

// Read returns the bytes named by ref and whether this namespace holds them.
// A malformed reference or missing blob returns absent, not an error.
func (n *Namespace) Read(ctx context.Context, ref core.BlobRef) (core.B64Bytes, bool, error) {
	panic("not written: b-blobs")
}

// List returns every blob in the namespace with its write time in one listing
// (on S3, one request per 1,000 objects). Names that are not blob names are omitted.
func (n *Namespace) List(ctx context.Context) ([]Stored, error) {
	panic("not written: b-blobs")
}

// DeleteUnused deletes ref unless something used it within grace, answering
// whether it was deleted. Checking uses and marking deletion are atomic: until
// deletion ends, a put waits and a commit that references the blob fails.
func (n *Namespace) DeleteUnused(ctx context.Context, ref core.BlobRef, grace time.Duration) (bool, error) {
	panic("not written: b-blobs")
}

// CommitUses records that a commit writes or removes references to refs, inside
// its transaction and before committing. If any blob is being deleted, it
// returns a store.Error of kind store.OperationFailed and the transaction must not commit.
func (n *Namespace) CommitUses(refs []core.BlobRef) error {
	panic("not written: b-blobs")
}

// ForgetUses forgets uses older than grace, which no longer hold deletion back.
func (n *Namespace) ForgetUses(grace time.Duration) {
	panic("not written: b-blobs")
}
