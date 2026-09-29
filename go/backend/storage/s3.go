package storage

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"time"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/config"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"github.com/aws/aws-sdk-go-v2/service/s3/types"
	"github.com/aws/smithy-go"
)

type S3Config struct {
	Bucket         string  `json:"bucket"`
	Region         string  `json:"region"`
	Endpoint       *string `json:"endpoint,omitzero"`
	ForcePathStyle bool    `json:"forcePathStyle,omitzero"`
}

func (c S3Config) Check() error {
	if c.Bucket == "" || c.Region == "" {
		return errors.New("bucket and region must not be empty")
	}
	if c.Endpoint != nil {
		endpoint, err := url.Parse(*c.Endpoint)
		if err != nil || endpoint.Scheme != "https" {
			return errors.New("the endpoint must be an HTTPS URL")
		}
	}
	return nil
}
func ReadS3Config(path string) (S3Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return S3Config{}, err
	}
	var cfg S3Config
	if err = json.Unmarshal(data, &cfg, json.RejectUnknownMembers(true)); err != nil {
		return cfg, err
	}
	return cfg, cfg.Check()
}

type S3Objects struct {
	client    *s3.Client
	presign   *s3.PresignClient
	bucket    string
	transport *http.Transport
}

func OpenS3Objects(ctx context.Context, c S3Config) (*S3Objects, error) {
	if err := c.Check(); err != nil {
		return nil, err
	}
	// Explicit empty file lists disable both default profile files. Environment,
	// web identity, container and instance credentials retain the SDK's providers.
	cfg, err := config.LoadDefaultConfig(ctx, config.WithRegion(c.Region), config.WithSharedConfigFiles([]string{}), config.WithSharedCredentialsFiles([]string{}))
	if err != nil {
		return nil, err
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	cfg.HTTPClient = &http.Client{Transport: transport}
	client := s3.NewFromConfig(cfg, func(o *s3.Options) {
		o.UsePathStyle = c.ForcePathStyle
		o.BaseEndpoint = c.Endpoint
	})
	return &S3Objects{client: client, presign: s3.NewPresignClient(client), bucket: c.Bucket, transport: transport}, nil
}
func (s *S3Objects) Close() error {
	if s.transport != nil {
		s.transport.CloseIdleConnections()
	}
	return nil
}
func s3Error(err error) error {
	var api smithy.APIError
	if errors.As(err, &api) {
		switch api.ErrorCode() {
		case "NotFound", "NoSuchKey":
			return ErrObjectNotFound
		case "PreconditionFailed":
			return ErrObjectExists
		}
	}
	return err
}
func (s *S3Objects) Head(ctx context.Context, key string) (ObjectMeta, error) {
	out, err := s.client.HeadObject(ctx, &s3.HeadObjectInput{Bucket: &s.bucket, Key: &key})
	if err != nil {
		return ObjectMeta{}, s3Error(err)
	}
	return ObjectMeta{Key: key, Size: aws.ToInt64(out.ContentLength), LastModified: aws.ToTime(out.LastModified), Metadata: out.Metadata}, nil
}
func (s *S3Objects) Get(ctx context.Context, key string) ([]byte, error) {
	out, err := s.client.GetObject(ctx, &s3.GetObjectInput{Bucket: &s.bucket, Key: &key, ChecksumMode: types.ChecksumModeEnabled})
	if err != nil {
		return nil, s3Error(err)
	}
	defer out.Body.Close()
	data, err := io.ReadAll(out.Body)
	if err != nil {
		return nil, err
	}
	if out.ChecksumSHA256 != nil {
		sum := sha256.Sum256(data)
		if base64.StdEncoding.EncodeToString(sum[:]) != *out.ChecksumSHA256 {
			return nil, errors.New("SHA-256 checksum mismatch")
		}
	}
	return data, nil
}
func (s *S3Objects) Create(ctx context.Context, key string, data []byte, metadata map[string]string) error {
	sum := sha256.Sum256(data)
	checksum := base64.StdEncoding.EncodeToString(sum[:])
	_, err := s.client.PutObject(ctx, &s3.PutObjectInput{Bucket: &s.bucket, Key: &key, Body: bytes.NewReader(data), IfNoneMatch: aws.String("*"), ChecksumAlgorithm: types.ChecksumAlgorithmSha256, ChecksumSHA256: &checksum, Metadata: metadata})
	return s3Error(err)
}
func (s *S3Objects) List(ctx context.Context, prefix string) ([]ObjectMeta, error) {
	pages := s3.NewListObjectsV2Paginator(s.client, &s3.ListObjectsV2Input{Bucket: &s.bucket, Prefix: &prefix})
	objects := []ObjectMeta{}
	for pages.HasMorePages() {
		page, err := pages.NextPage(ctx)
		if err != nil {
			return nil, err
		}
		for _, entry := range page.Contents {
			objects = append(objects, ObjectMeta{Key: aws.ToString(entry.Key), Size: aws.ToInt64(entry.Size), LastModified: aws.ToTime(entry.LastModified)})
		}
	}
	return objects, nil
}
func (s *S3Objects) Delete(ctx context.Context, key string) error {
	_, err := s.client.DeleteObject(ctx, &s3.DeleteObjectInput{Bucket: &s.bucket, Key: &key})
	return s3Error(err)
}
func (s *S3Objects) PresignGet(ctx context.Context, key string, lifetime time.Duration) (string, error) {
	signed, err := s.presign.PresignGetObject(ctx, &s3.GetObjectInput{Bucket: &s.bucket, Key: &key}, func(o *s3.PresignOptions) { o.Expires = lifetime })
	if err != nil {
		return "", fmt.Errorf("presigning GET: %w", err)
	}
	return signed.URL, nil
}
