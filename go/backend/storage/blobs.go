package storage

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"path"
	"strings"
	"sync"
	"time"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type blobKey struct {
	user webapi.UserID
	blob core.BlobRef
}
type BlobStores struct {
	objects  ObjectStore
	clock    core.Clock
	mu       sync.Mutex
	last     map[blobKey]int64
	deleting map[blobKey]chan struct{}
}
type UserBlobs struct {
	stores *BlobStores
	user   webapi.UserID
}
type StoredBlob struct {
	Blob    core.BlobRef
	Written core.Timestamp
}

func NewBlobStores(objects ObjectStore, clock core.Clock) *BlobStores {
	return &BlobStores{objects: objects, clock: clock, last: map[blobKey]int64{}, deleting: map[blobKey]chan struct{}{}}
}
func (s *BlobStores) ForUser(user webapi.UserID) *UserBlobs { return &UserBlobs{s, user} }

// namespace uses object_store::PathPart's byte escaping, including dot segments.
func (b *UserBlobs) namespace() string {
	name := b.user.String()
	if name == "." {
		name = "%2E"
	} else if name == ".." {
		name = "%2E%2E"
	} else {
		var escaped strings.Builder
		const digits = "0123456789ABCDEF"
		for _, value := range []byte(name) {
			if value < 32 || value >= 127 || strings.ContainsRune("/\\{^}%`]\">[~<#|*?", rune(value)) {
				escaped.WriteByte('%')
				escaped.WriteByte(digits[value>>4])
				escaped.WriteByte(digits[value&15])
			} else {
				escaped.WriteByte(value)
			}
		}
		name = escaped.String()
	}
	return "blobs/" + name + "/"
}
func (b *UserBlobs) location(blob core.BlobRef) string { return b.namespace() + blob.String() }
func (b *UserBlobs) Put(ctx context.Context, data []byte) (core.BlobRef, error) {
	sum := sha256.Sum256(data)
	blob, err := core.ParseBlobRef(hex.EncodeToString(sum[:]))
	if err != nil {
		return blob, err
	}
	s := b.stores
	key := blobKey{b.user, blob}
	for {
		s.mu.Lock()
		done := s.deleting[key]
		if done == nil {
			s.last[key] = s.clock.Now().Millisecond()
			s.mu.Unlock()
			break
		}
		s.mu.Unlock()
		select {
		case <-done:
		case <-ctx.Done():
			return blob, ctx.Err()
		}
	}
	_, err = s.objects.Head(ctx, b.location(blob))
	if err == nil {
		return blob, nil
	}
	if !errors.Is(err, ErrObjectNotFound) {
		return blob, objectError(err)
	}
	err = s.objects.Create(ctx, b.location(blob), data, nil)
	if errors.Is(err, ErrObjectExists) {
		err = nil
	}
	return blob, objectError(err)
}
func (b *UserBlobs) Get(ctx context.Context, blob core.BlobRef) ([]byte, error) {
	if err := core.Validate(blob); err != nil {
		return nil, nil
	}
	data, err := b.stores.objects.Get(ctx, b.location(blob))
	if errors.Is(err, ErrObjectNotFound) {
		return nil, nil
	}
	return data, objectError(err)
}
func (b *UserBlobs) List(ctx context.Context) ([]StoredBlob, error) {
	objects, err := b.stores.objects.List(ctx, b.namespace())
	if err != nil {
		return nil, objectError(err)
	}
	blobs := []StoredBlob{}
	for _, object := range objects {
		blob, err := core.ParseBlobRef(path.Base(object.Key))
		if err != nil {
			continue
		}
		written, err := instant("blobs", "last_modified", object.LastModified.UnixMilli())
		if err != nil {
			return nil, err
		}
		blobs = append(blobs, StoredBlob{blob, written})
	}
	return blobs, nil
}
func (b *UserBlobs) CommitUses(blobs []core.BlobRef) error {
	s := b.stores
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, blob := range blobs {
		if s.deleting[blobKey{b.user, blob}] != nil {
			return fmt.Errorf("blob %s is being deleted", blob.String())
		}
	}
	now := s.clock.Now().Millisecond()
	for _, blob := range blobs {
		s.last[blobKey{b.user, blob}] = now
	}
	return nil
}
func (b *UserBlobs) DeleteUnused(ctx context.Context, blob core.BlobRef, grace time.Duration) (bool, error) {
	s := b.stores
	key := blobKey{b.user, blob}
	s.mu.Lock()
	used, ok := s.last[key]
	if s.deleting[key] != nil || (ok && s.clock.Now().Millisecond()-used <= grace.Milliseconds()) {
		s.mu.Unlock()
		return false, nil
	}
	done := make(chan struct{})
	s.deleting[key] = done
	s.mu.Unlock()
	defer func() {
		s.mu.Lock()
		delete(s.deleting, key)
		close(done)
		s.mu.Unlock()
	}()
	err := s.objects.Delete(ctx, b.location(blob))
	if errors.Is(err, ErrObjectNotFound) {
		err = nil
	}
	return err == nil, objectError(err)
}
func (b *UserBlobs) ForgetUses(grace time.Duration) {
	s := b.stores
	s.mu.Lock()
	defer s.mu.Unlock()
	now := s.clock.Now().Millisecond()
	for key, used := range s.last {
		if key.user == b.user && now-used > grace.Milliseconds() {
			delete(s.last, key)
		}
	}
}
