package blobs

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"strings"

	"github.com/aws/aws-sdk-go-v2/aws"
	awshttp "github.com/aws/aws-sdk-go-v2/aws/transport/http"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"github.com/wspl/demi/internal/contract"
	"gocloud.dev/blob"
	"gocloud.dev/blob/s3blob"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

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
	if err := ctx.Err(); err != nil {
		return S3Config{}, &ConfigError{Err: err}
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return S3Config{}, &ConfigError{Err: err}
	}
	value, err := DecodeS3Config(data)
	if err != nil {
		return S3Config{}, &ConfigError{Err: err}
	}
	if value.Endpoint != nil {
		endpoint, err := s3Endpoint(*value.Endpoint)
		if err != nil {
			return S3Config{}, &ConfigError{Err: err}
		}
		value.Endpoint = &endpoint
	}
	return value, nil
}

// Open opens the bucket's client, which checksums every upload with SHA-256
// and supports conditional creation through blob.WriterOptions.IfNotExist.
// Credentials come from AWS environment variables, web identity, or container
// or instance metadata, never shared credentials or profile files. The caller
// must close the returned bucket.
func (c S3Config) Open(ctx context.Context) (*blob.Bucket, error) {
	if err := c.Validate(); err != nil {
		return nil, &ConfigError{Err: err}
	}
	client := &http.Client{
		Transport:     awshttp.NewBuildableClient().GetTransport(),
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
	provider, err := storageCredentials(ctx, c.Region, client)
	if err != nil {
		client.CloseIdleConnections()
		return nil, &Error{Err: err}
	}
	endpoint := c.Endpoint
	if endpoint != nil {
		normalized, err := s3Endpoint(*endpoint)
		if err != nil {
			client.CloseIdleConnections()
			return nil, &ConfigError{Err: err}
		}
		normalized = strings.TrimRight(normalized, "/")
		endpoint = &normalized
	}
	lifetime, cancel := context.WithCancel(context.Background())
	owner := &credentialLifetime{provider: provider, ctx: lifetime, cancel: cancel}
	service := s3.New(s3.Options{
		Region: c.Region, HTTPClient: client, Credentials: aws.NewCredentialsCache(owner),
		BaseEndpoint: endpoint, UsePathStyle: c.ForcePathStyle,
	})
	bucket, err := s3blob.OpenBucket(ctx, service, c.Bucket, nil)
	if err != nil {
		owner.close()
		client.CloseIdleConnections()
		return nil, &Error{Err: err}
	}
	return blob.NewBucket(&s3Bucket{Bucket: bucket, closeTransport: func() {
		owner.close()
		client.CloseIdleConnections()
	}}), nil
}

// checkS3Config checks the bucket, region and HTTPS endpoint after shape decoding.
func checkS3Config(c S3Config) error {
	if c.Bucket == "" || c.Region == "" {
		return fmt.Errorf("bucket and region must not be empty")
	}
	if c.Endpoint != nil {
		_, err := s3Endpoint(*c.Endpoint)
		return err
	}
	return nil
}

// s3Endpoint preserves Rust URL parsing and restricts object-store endpoints to HTTPS.
func s3Endpoint(value string) (string, error) {
	endpoint, err := contract.HTTPURL(value)
	if err != nil {
		return "", err
	}
	if !strings.HasPrefix(endpoint, "https://") {
		return "", fmt.Errorf("the endpoint must be an HTTPS URL")
	}
	return endpoint, nil
}
