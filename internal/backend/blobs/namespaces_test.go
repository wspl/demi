package blobs_test

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/google/go-cmp/cmp"
	"go.uber.org/goleak"
	"gocloud.dev/blob"
	"gocloud.dev/blob/memblob"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
	"github.com/wspl/demi/internal/core"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

type wallClock struct{ milliseconds atomic.Int64 }

func (c *wallClock) Now() core.Timestamp {
	value, err := core.TimestampFromMillisecond(c.milliseconds.Load())
	if err != nil {
		panic(err)
	} // Tests only assign the Unix epoch and a few days after it.
	return value
}

const day = 24 * time.Hour

type heldDeletion struct {
	blobs.Objects
	reached chan struct{}
	proceed chan struct{}
	err     error
}

func (h *heldDeletion) Delete(ctx context.Context, key string) error {
	close(h.reached)
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-h.proceed:
	}
	if h.err != nil {
		return h.err
	}
	return h.Objects.Delete(ctx, key)
}

type putResult struct {
	ref core.BlobRef
	err error
}
type deleteResult struct {
	deleted bool
	err     error
}

// The Rust deletion interleaving runs entirely in virtual time, with no network.
func TestPutWaitsForDeletionAndStoresAgain(t *testing.T) {
	for _, deletionFails := range []bool{false, true} {
		t.Run(fmt.Sprintf("deletionFails=%t", deletionFails), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				ctx, cancel := context.WithCancel(t.Context())
				defer cancel()
				bucket := memblob.OpenBucket(nil)
				defer func() {
					if err := bucket.Close(); err != nil {
						t.Error(err)
					}
				}()
				var workers sync.WaitGroup
				defer func() {
					cancel()
					workers.Wait()
				}()
				held := &heldDeletion{Objects: bucket, reached: make(chan struct{}), proceed: make(chan struct{})}
				failed := errors.New("delete failed")
				if deletionFails {
					held.err = failed
				}
				clock := &wallClock{}
				namespaces := blobs.New(held, clock)
				namespace := namespaces.ForUser("ana")
				data := core.B64Bytes("a screenshot")
				ref, err := namespace.Put(ctx, data)
				if err != nil {
					t.Fatal(err)
				}
				if deleted, err := namespace.DeleteUnused(ctx, ref, day); err != nil || deleted {
					t.Fatalf("recent deletion = %t, %v", deleted, err)
				}
				clock.milliseconds.Store((2 * day).Milliseconds())
				deletion := make(chan deleteResult, 1)
				workers.Go(func() {
					deleted, err := namespace.DeleteUnused(ctx, ref, day)
					deletion <- deleteResult{deleted, err}
				})
				<-held.reached
				if err := namespace.CommitUses([]core.BlobRef{ref}); !errors.Is(err, &store.Error{Kind: store.OperationFailed}) {
					t.Fatalf("commit during deletion = %v", err)
				}
				if deleted, err := namespace.DeleteUnused(ctx, ref, day); deleted || err != nil {
					t.Fatalf("overlapping deletion = %t, %v", deleted, err)
				}
				put := make(chan putResult, 1)
				// A fresh handle must share the same deletion record.
				workers.Go(func() {
					ref, err := namespaces.ForUser("ana").Put(ctx, data)
					put <- putResult{ref, err}
				})
				synctest.Wait()
				select {
				case result := <-put:
					t.Fatalf("put escaped deletion: %+v", result)
				default:
				}
				canceled, stop := context.WithCancel(ctx)
				canceledPut := make(chan error, 1)
				workers.Go(func() {
					_, err := namespace.Put(canceled, data)
					canceledPut <- err
				})
				synctest.Wait()
				stop()
				if err := <-canceledPut; !errors.Is(err, context.Canceled) {
					t.Fatalf("canceled waiting put = %v", err)
				}

				close(held.proceed)
				result := <-deletion
				if deletionFails {
					if !errors.Is(result.err, failed) || result.deleted {
						t.Fatalf("deletion = %+v", result)
					}
				} else if result.err != nil || !result.deleted {
					t.Fatalf("deletion = %+v", result)
				}
				stored := <-put
				if stored.err != nil || stored.ref != ref {
					t.Fatalf("put = %+v", stored)
				}
				got, exists, err := namespace.Read(ctx, ref)
				if err != nil || !exists || !bytes.Equal(got, data) {
					t.Fatalf("read = %q, %t, %v", got, exists, err)
				}
				if err := namespace.CommitUses([]core.BlobRef{ref}); err != nil {
					t.Fatal(err)
				}
			})
		})
	}
}

// staleHead forces the check/create race: the real conditional write still runs.
type staleHead struct{ blobs.Objects }

func (s staleHead) Attributes(ctx context.Context, _ string) (*blob.Attributes, error) {
	return s.Objects.Attributes(ctx, "missing-object")
}

// Exercise both production storage drivers through the same public namespace API.
func TestS3BucketHoldsEachBlobOnceUnderUserNamespace(t *testing.T) {
	for _, backend := range []string{"file", "s3"} {
		t.Run(backend, func(t *testing.T) {
			ctx := t.Context()
			var bucket *blob.Bucket
			var fake *blobstest.FakeS3
			var err error
			if backend == "file" {
				bucket, err = blobs.Open(ctx, t.TempDir(), nil)
			} else {
				fake, err = blobstest.StartS3(ctx)
				if err != nil {
					t.Fatal(err)
				}
				t.Cleanup(func() {
					if err := fake.Close(context.Background()); err != nil {
						t.Error(err)
					}
				})
				bucket, err = fake.Client(ctx)
			}
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() {
				if err := bucket.Close(); err != nil {
					t.Error(err)
				}
			})
			counts := &blobstest.ObjectCounts{}
			clock := &wallClock{}
			stores := blobs.New(counts.Observe(bucket), clock)
			ana, other := stores.ForUser("ana"), stores.ForUser("another")
			data := core.B64Bytes("picture")
			ref, err := ana.Put(ctx, data)
			if err != nil {
				t.Fatal(err)
			}
			again, err := ana.Put(ctx, data)
			if err != nil || ref != again {
				t.Fatalf("repeated put = %s, %v", again, err)
			}
			if got := counts.Tally(); got.Puts != 1 || got.BytesPut != uint64(len(data)) || got.Heads != 2 {
				t.Fatalf("tally = %+v", got)
			}
			got, exists, err := ana.Read(ctx, ref)
			if err != nil || !exists || !bytes.Equal(got, data) {
				t.Fatalf("read = %q, %t, %v", got, exists, err)
			}
			if _, exists, err := other.Read(ctx, ref); err != nil || exists {
				t.Fatalf("other user's read = %t, %v", exists, err)
			}
			if _, exists, err := ana.Read(ctx, "../invalid"); err != nil || exists {
				t.Fatalf("malformed read = %t, %v", exists, err)
			}
			before := counts.Tally()
			// Return an actual missing HEAD response even though the object now exists.
			raced := blobs.New(counts.Observe(staleHead{bucket}), clock).ForUser("ana")
			if same, err := raced.Put(ctx, data); err != nil || same != ref {
				t.Fatalf("racing create = %s, %v", same, err)
			}
			if delta := counts.Tally().Since(before); delta.Puts != 1 || delta.Heads != 1 {
				t.Fatalf("race tally = %+v", delta)
			}
			if fake != nil {
				if diff := cmp.Diff([]string{"blobs/ana/" + string(ref)}, fake.Written()); diff != "" {
					t.Fatal(diff)
				}
			}
			if err := bucket.WriteAll(ctx, "blobs/ana/not-a-hash", []byte("leave alone"), nil); err != nil {
				t.Fatal(err)
			}
			listed, err := ana.List(ctx)
			if err != nil || len(listed) != 1 || listed[0].Blob != ref {
				t.Fatalf("listing = %+v, %v", listed, err)
			}
			if err := listed[0].Written.Validate(); err != nil {
				t.Fatal(err)
			}
			clock.milliseconds.Store(day.Milliseconds())
			ana.ForgetUses(day)
			if deleted, err := ana.DeleteUnused(ctx, ref, day); err != nil || deleted {
				t.Fatalf("grace boundary = %t, %v", deleted, err)
			}
			if err := ana.CommitUses([]core.BlobRef{ref}); err != nil {
				t.Fatal(err)
			}
			clock.milliseconds.Store((2 * day).Milliseconds())
			if deleted, err := ana.DeleteUnused(ctx, ref, day); err != nil || deleted {
				t.Fatalf("commit grace = %t, %v", deleted, err)
			}
			clock.milliseconds.Add(1)
			ana.ForgetUses(day)
			if deleted, err := ana.DeleteUnused(ctx, ref, day); err != nil || !deleted {
				t.Fatalf("old deletion = %t, %v", deleted, err)
			}
			if _, exists, err := ana.Read(ctx, ref); err != nil || exists {
				t.Fatalf("deleted read = %t, %v", exists, err)
			}
			if deleted, err := ana.DeleteUnused(ctx, ref, day); err != nil || !deleted {
				t.Fatalf("missing deletion = %t, %v", deleted, err)
			}
			empty, err := ana.Put(ctx, nil)
			if err != nil {
				t.Fatal(err)
			}
			if data, found, err := ana.Read(ctx, empty); err != nil || !found || len(data) != 0 {
				t.Fatalf("empty blob = %q, %t, %v", data, found, err)
			}

		})
	}
}

func TestS3ConfigurationNamesBucketAndRegionOverHTTPS(t *testing.T) {
	path := filepath.Join(t.TempDir(), "s3.json")
	cases := []struct {
		name, text string
		valid      bool
	}{
		{"full", `{"bucket":"demi","region":"eu-west-1","endpoint":"https://objects.example.com","forcePathStyle":true}`, true},
		{"defaults", `{"bucket":"demi","region":"eu-west-1"}`, true},
		{"null endpoint", `{"bucket":"demi","region":"eu-west-1","endpoint":null}`, true},
		{"http", `{"bucket":"demi","region":"eu-west-1","endpoint":"http://objects.example.com"}`, false},
		{"empty bucket", `{"bucket":"","region":"eu-west-1"}`, false},
		{"empty region", `{"bucket":"demi","region":""}`, false},
		{"unknown", `{"bucket":"demi","region":"eu-west-1","profile":"default"}`, false},
		{"missing", `{"region":"eu-west-1"}`, false},
		{"invalid URL", `{"bucket":"demi","region":"eu-west-1","endpoint":"https://%zz"}`, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if err := os.WriteFile(path, []byte(tc.text), 0600); err != nil {
				t.Fatal(err)
			}
			config, err := blobs.ReadS3Config(t.Context(), path)
			if (err == nil) != tc.valid {
				t.Fatalf("configuration = %+v, %v", config, err)
			}
			if tc.name == "full" && !config.ForcePathStyle {
				t.Fatal("path style lost")
			}
		})
	}
	if _, err := blobs.ReadS3Config(t.Context(), path+"missing"); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("read error = %v", err)
	}
}

type heldReads struct {
	blobs.Objects
	entered chan struct{}
	release chan struct{}
	failure error
}

func (h heldReads) ReadAll(ctx context.Context, _ string) ([]byte, error) {
	h.entered <- struct{}{}
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-h.release:
		return nil, h.failure
	}
}

func TestObjectCountsTracksConcurrentFailedReads(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		var workers sync.WaitGroup
		defer func() {
			cancel()
			workers.Wait()
		}()
		failed := errors.New("read failed")
		held := heldReads{entered: make(chan struct{}, 2), release: make(chan struct{}), failure: failed}
		counts := &blobstest.ObjectCounts{}
		objects := counts.Observe(held)
		ended := make(chan error, 2)
		for range 2 {
			workers.Go(func() {
				_, err := objects.ReadAll(ctx, "key")
				ended <- err
			})
		}
		for range 2 {
			<-held.entered
		}
		if tally := counts.Tally(); tally.Gets != 2 || tally.MostGetsAtOnce != 2 {
			t.Fatalf("in flight = %+v", tally)
		}
		close(held.release)
		for range 2 {
			if err := <-ended; !errors.Is(err, failed) {
				t.Fatal(err)
			}
		}
		if _, err := objects.ReadAll(ctx, "key"); !errors.Is(err, failed) {
			t.Fatal(err)
		}
		if tally := counts.Tally(); tally.Gets != 3 || tally.MostGetsAtOnce != 2 {
			t.Fatalf("after failures = %+v", tally)
		}
	})
}
