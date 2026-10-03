package blobs

import (
	"context"
	"fmt"

	"gocloud.dev/blob"
	"gocloud.dev/blob/fileblob"
)

// Objects is the object-store boundary used by blob namespaces. A *blob.Bucket
// implements it for both fileblob and s3blob. Implementations support concurrent
// calls and retain the driver's gcerrors codes. The caller owns the bucket and
// closes it after all namespaces have stopped using it.
type Objects interface {
	// Attributes reads metadata without transferring the object's bytes.
	Attributes(ctx context.Context, key string) (*blob.Attributes, error)
	// ReadAll reads the object's bytes.
	ReadAll(ctx context.Context, key string) ([]byte, error)
	// WriteAll writes bytes with the supplied metadata and creation condition.
	WriteAll(ctx context.Context, key string, data []byte, opts *blob.WriterOptions) error
	// List starts a listing; its iterator's Next accepts the wait's context.
	List(opts *blob.ListOptions) *blob.ListIterator
	// Delete removes the object at key.
	Delete(ctx context.Context, key string) error
}

// Open opens the S3 bucket s3 names, or the existing data directory when s3
// is nil. The caller must close the returned bucket at shutdown.
func Open(ctx context.Context, dataDir string, s3 *S3Config) (*blob.Bucket, error) {
	if err := ctx.Err(); err != nil {
		return nil, fmt.Errorf("the object store failed: %w", err)
	}
	if s3 != nil {
		return s3.Open(ctx)
	}
	bucket, err := fileblob.OpenBucket(dataDir, nil)
	if err != nil {
		return nil, fmt.Errorf("the object store failed: %w", err)
	}
	return bucket, nil
}
