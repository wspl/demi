// Named parameters document this API checkpoint; bodies follow after its merge.
//revive:disable:unused-parameter

package blobs

import (
	"context"

	"gocloud.dev/blob"
)

//go:generate go run ../../../tools/contractgen .

// Contract documentation is copied verbatim from Rust.
//revive:disable:exported

// Where `DEMI_OBJECT_STORE_CONFIG` puts the object store: an S3 bucket.
//
// +demi:root
// +demi:check checkS3Config
type S3Config struct {
	Bucket string `json:"bucket"`
	Region string `json:"region"`
	// An S3-compatible service instead of AWS, over HTTPS.
	// +demi:nullable
	Endpoint *string `json:"endpoint,omitempty"`
	// Names the bucket in the request path instead of the host name.
	// +demi:default
	ForcePathStyle bool `json:"forcePathStyle"`
}

//revive:enable:exported

// ReadS3Config reads and checks the configuration in the JSON file at path.
func ReadS3Config(ctx context.Context, path string) (S3Config, error) {
	panic("not written: b-blobs")
}

// Open opens the bucket's client, which checksums every upload with SHA-256
// and supports conditional creation through blob.WriterOptions.IfNotExist.
// Credentials come from AWS environment variables, web identity, or container
// or instance metadata, never shared credentials or profile files. The caller
// must close the returned bucket.
func (c S3Config) Open(ctx context.Context) (*blob.Bucket, error) {
	panic("not written: b-blobs")
}

// checkS3Config checks the bucket, region and HTTPS endpoint after shape decoding.
func checkS3Config(S3Config) error {
	panic("not written: b-blobs")
}
