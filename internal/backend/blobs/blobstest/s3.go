// Named parameters document this API checkpoint; bodies follow after its merge.
//revive:disable:unused-parameter

package blobstest

import (
	"context"

	"gocloud.dev/blob"

	"github.com/wspl/demi/internal/backend/blobs"
)

// FakeS3 is an S3-compatible service in memory. It handles puts, conditional
// creation, reads with metadata, and deletions without checking signatures.
// Its owner must close it after its clients stop using it.
type FakeS3 struct {
	// Endpoint is the local HTTP URL at which the fake accepts requests.
	Endpoint string
}

// StartS3 starts a fake on a loopback port allocated by the operating system.
// The caller owns it and must call Close, which stops and joins its server.
func StartS3(ctx context.Context) (*FakeS3, error) {
	panic("not written: b-blobs")
}

// Close stops and joins the fake's server and releases its listener and connections.
func (f *FakeS3) Close(ctx context.Context) error {
	panic("not written: b-blobs")
}

// Config returns the fake's bucket as the backend's configuration names it.
// Its HTTP endpoint is test-only and does not pass production HTTPS validation.
func (f *FakeS3) Config() blobs.S3Config {
	panic("not written: b-blobs")
}

// Client opens the backend's client of the fake, with fake credentials and
// plain HTTP permitted. The caller must close the returned bucket.
func (f *FakeS3) Client(ctx context.Context) (*blob.Bucket, error) {
	panic("not written: b-blobs")
}

// Written returns the keys of objects put, in the order they were put.
func (f *FakeS3) Written() []string {
	panic("not written: b-blobs")
}

// Object returns a copy of the bytes at key and whether the object exists.
func (f *FakeS3) Object(key string) ([]byte, bool) {
	panic("not written: b-blobs")
}
