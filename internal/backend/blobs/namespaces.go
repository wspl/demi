package blobs

import (
	"context"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"path"
	"sync"
	"time"

	"gocloud.dev/blob"
	"gocloud.dev/gcerrors"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Stores holds every user's blob namespace, with the backend's record of blob uses.
// Its namespaces share the use record and support concurrent calls.
type Stores struct {
	objects Objects
	clock   core.Clock
	// mu protects last and deleting; no IO or notification happens under it.
	mu       sync.Mutex
	last     map[webapi.UserID]map[core.BlobRef]core.Timestamp
	deleting map[blobKey]chan struct{}
}

type blobKey struct {
	user webapi.UserID
	ref  core.BlobRef
}

// New returns the namespaces in objects, whose uses are timed by clock.
// It borrows objects; its caller retains responsibility for closing the bucket.
func New(objects Objects, clock core.Clock) *Stores {
	return &Stores{
		objects:  objects,
		clock:    clock,
		last:     make(map[webapi.UserID]map[core.BlobRef]core.Timestamp),
		deleting: make(map[blobKey]chan struct{}),
	}
}

// ForUser returns user's namespace: the signed-in user's for uploads and
// downloads, the conversation owner's for transcript media.
func (s *Stores) ForUser(user webapi.UserID) *Namespace {
	return &Namespace{stores: s, user: user}
}

// Namespace holds one user's blobs and implements the session's store.BlobStore.
// It borrows its store and supports concurrent calls.
type Namespace struct {
	stores *Stores
	user   webapi.UserID
}

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
	ref := core.BlobRefOf(data)
	if err := n.recordPut(ctx, ref); err != nil {
		return "", &Error{Err: err}
	}
	key := n.prefix() + string(ref)
	if _, err := n.stores.objects.Attributes(ctx, key); err == nil {
		return ref, nil
	} else if gcerrors.Code(err) != gcerrors.NotFound {
		return "", &Error{Err: err}
	}
	// BlobRefOf constructed this hex digest here, so decoding it cannot fail.
	digest, _ := hex.DecodeString(string(ref))
	checksum := base64.StdEncoding.EncodeToString(digest)
	opts := &blob.WriterOptions{
		IfNotExist:                  true,
		DisableContentTypeDetection: true,
		BeforeWrite: func(as func(any) bool) error {
			return sha256Upload(as, &checksum)
		},
	}
	if err := n.stores.objects.WriteAll(
		ctx,
		key,
		data,
		opts,
	); err != nil && gcerrors.Code(err) != gcerrors.FailedPrecondition &&
		gcerrors.Code(err) != gcerrors.AlreadyExists {
		return "", &Error{Err: err}
	}
	return ref, nil
}

// Read returns the bytes named by ref and whether this namespace holds them.
// A malformed reference or missing blob returns absent, not an error.
func (n *Namespace) Read(ctx context.Context, ref core.BlobRef) (core.B64Bytes, bool, error) {
	if err := ref.Validate(); err != nil {
		return nil, false, nil
	}
	data, err := n.stores.objects.ReadAll(ctx, n.prefix()+string(ref))
	if gcerrors.Code(err) == gcerrors.NotFound {
		return nil, false, nil
	}
	if err != nil {
		return nil, false, &Error{Err: err}
	}
	return core.B64Bytes(data), true, nil
}

// List returns every blob in the namespace with its write time in one listing
// (on S3, one request per 1,000 objects). Names that are not blob names are omitted.
func (n *Namespace) List(ctx context.Context) ([]Stored, error) {
	listing := n.stores.objects.List(&blob.ListOptions{Prefix: n.prefix()})
	result := []Stored{}
	for {
		object, err := listing.Next(ctx)
		if errors.Is(err, io.EOF) {
			return result, nil
		}
		if err != nil {
			return nil, &Error{Err: err}
		}
		ref, err := core.ParseBlobRef(path.Base(object.Key))
		if err != nil {
			slog.Warn("an object in a blob namespace is not a blob", "object", object.Key)
			continue
		}
		written, err := core.TimestampFromTime(object.ModTime)
		if err != nil {
			return nil, &CorruptError{Location: object.Key, Field: "last modified time", Reason: err.Error()}
		}
		result = append(result, Stored{Blob: ref, Written: written})
	}
}

// DeleteUnused deletes ref unless something used it within grace, answering
// whether it was deleted. Checking uses and marking deletion are atomic: until
// deletion ends, a put waits and a commit that references the blob fails.
func (n *Namespace) DeleteUnused(ctx context.Context, ref core.BlobRef, grace time.Duration) (bool, error) {
	s := n.stores
	now := s.clock.Now()
	key := blobKey{n.user, ref}
	s.mu.Lock()
	used, found := s.last[n.user][ref]
	if (found && recentUse(now, used, grace)) || s.deleting[key] != nil {
		s.mu.Unlock()
		return false, nil
	}
	ended := make(chan struct{})
	s.deleting[key] = ended
	s.mu.Unlock()
	defer func() {
		s.mu.Lock()
		delete(s.deleting, key)
		s.mu.Unlock()
		close(ended)
	}()
	err := s.objects.Delete(ctx, n.prefix()+string(ref))
	if err != nil && gcerrors.Code(err) != gcerrors.NotFound {
		return false, &Error{Err: err}
	}
	return true, nil
}

// CommitUses records that a commit writes or removes references to refs, inside
// its transaction and before committing. If any blob is being deleted, it
// returns an error and the transaction must not commit.
func (n *Namespace) CommitUses(refs []core.BlobRef) error {
	if len(refs) == 0 {
		return nil
	}
	s := n.stores
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, ref := range refs {
		if s.deleting[blobKey{n.user, ref}] != nil {
			return fmt.Errorf("blob %s is being deleted", ref)
		}
	}
	now := s.clock.Now()
	for _, ref := range refs {
		n.recordUseLocked(ref, now)
	}
	return nil
}

// ForgetUses forgets uses older than grace, which no longer hold deletion back.
func (n *Namespace) ForgetUses(grace time.Duration) {
	s := n.stores
	now := s.clock.Now()
	s.mu.Lock()
	defer s.mu.Unlock()
	for ref, used := range s.last[n.user] {
		if !recentUse(now, used, grace) {
			delete(s.last[n.user], ref)
		}
	}
	if len(s.last[n.user]) == 0 {
		delete(s.last, n.user)
	}
}

// recordPut registers this blob's use once any deletion has finished.
func (n *Namespace) recordPut(ctx context.Context, ref core.BlobRef) error {
	s := n.stores
	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		s.mu.Lock()
		ended := s.deleting[blobKey{n.user, ref}]
		if ended == nil {
			n.recordUseLocked(ref, s.clock.Now())
			s.mu.Unlock()
			return nil
		}
		s.mu.Unlock()
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ended:
		}
	}
}

// recordUseLocked updates a user's blob-use record while its store's mutex is held.
func (n *Namespace) recordUseLocked(ref core.BlobRef, now core.Timestamp) {
	if n.stores.last[n.user] == nil {
		n.stores.last[n.user] = make(map[core.BlobRef]core.Timestamp)
	}
	n.stores.last[n.user][ref] = now
}

// recentUse tests the blob retention grace, conservatively keeping invalid clock values.
func recentUse(now, used core.Timestamp, grace time.Duration) bool {
	current, err := now.Time()
	if err != nil {
		return true
	}
	previous, err := used.Time()
	if err != nil {
		return true
	}
	return !current.After(previous.Add(grace))
}

// prefix locates this user's objects without sharing another user's namespace.
func (n *Namespace) prefix() string { return "blobs/" + string(n.user) + "/" }
