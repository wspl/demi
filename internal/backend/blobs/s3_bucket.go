package blobs

import (
	"context"
	"fmt"
	"math"

	"github.com/aws/aws-sdk-go-v2/feature/s3/transfermanager"
	"github.com/aws/aws-sdk-go-v2/feature/s3/transfermanager/types"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"gocloud.dev/blob"
	"gocloud.dev/blob/driver"
	"gocloud.dev/gcerrors"
)

// s3Bucket forwards storage to Go Cloud while owning the S3 HTTP transport and
// the SHA-256 upload policy. s3blob exposes neither its driver nor a close hook,
// and its driver's Close is a no-op, so this adapter bridges its portable API
// back to a driver without implementing any S3 operations itself.
type s3Bucket struct {
	*blob.Bucket
	closeTransport func()
}

func (b *s3Bucket) Close() error {
	defer b.closeTransport()
	return b.Bucket.Close()
}

func (*s3Bucket) ErrorCode(err error) gcerrors.ErrorCode { return gcerrors.Code(err) }

func (b *s3Bucket) Attributes(ctx context.Context, key string) (*driver.Attributes, error) {
	a, err := b.Bucket.Attributes(ctx, key)
	if err != nil {
		return nil, err
	}
	return &driver.Attributes{CacheControl: a.CacheControl, ContentDisposition: a.ContentDisposition,
		ContentEncoding: a.ContentEncoding, ContentLanguage: a.ContentLanguage, ContentType: a.ContentType,
		Metadata: a.Metadata, CreateTime: a.CreateTime, ModTime: a.ModTime, Size: a.Size,
		MD5: a.MD5, ETag: a.ETag, AsFunc: a.As}, nil
}

func (b *s3Bucket) ListPaged(ctx context.Context, opts *driver.ListOptions) (*driver.ListPage, error) {
	objects, token, err := b.ListPage(ctx, opts.PageToken, opts.PageSize,
		&blob.ListOptions{Prefix: opts.Prefix, Delimiter: opts.Delimiter, BeforeList: opts.BeforeList})
	if err != nil {
		return nil, err
	}
	page := &driver.ListPage{NextPageToken: token}
	for _, o := range objects {
		page.Objects = append(page.Objects, &driver.ListObject{Key: o.Key, ModTime: o.ModTime,
			Size: o.Size, MD5: o.MD5, IsDir: o.IsDir, AsFunc: o.As})
	}
	return page, nil
}

type s3Reader struct{ *blob.Reader }

func (r *s3Reader) Attributes() *driver.ReaderAttributes {
	return &driver.ReaderAttributes{ContentType: r.ContentType(), ModTime: r.ModTime(), Size: r.Size()}
}

func (b *s3Bucket) NewRangeReader(ctx context.Context, key string, offset, length int64, opts *driver.ReaderOptions) (driver.Reader, error) {
	reader, err := b.Bucket.NewRangeReader(ctx, key, offset, length, &blob.ReaderOptions{BeforeRead: opts.BeforeRead})
	if err != nil {
		return nil, err
	}
	return &s3Reader{reader}, nil
}

func (b *s3Bucket) NewTypedWriter(ctx context.Context, key, contentType string, opts *driver.WriterOptions) (driver.Writer, error) {
	return b.NewWriter(ctx, key, &blob.WriterOptions{
		BufferSize: opts.BufferSize, MaxConcurrency: opts.MaxConcurrency, CacheControl: opts.CacheControl,
		ContentDisposition: opts.ContentDisposition, ContentEncoding: opts.ContentEncoding,
		ContentLanguage: opts.ContentLanguage, ContentType: contentType, ContentMD5: opts.ContentMD5,
		Metadata: opts.Metadata, DisableContentTypeDetection: opts.DisableContentTypeDetection,
		IfNotExist: opts.IfNotExist,
		BeforeWrite: func(as func(any) bool) error {
			// s3blob documents mutation of its transfer manager via BeforeWrite.
			// Rust's byte put is a single PUT, not a multipart upload at 16 MiB.
			var manager *transfermanager.Client
			var service *s3.Client
			if !as(&manager) || !b.As(&service) {
				return fmt.Errorf("the S3 driver did not expose its upload client")
			}
			*manager = *transfermanager.New(service, func(o *transfermanager.Options) {
				o.MultipartUploadThreshold = math.MaxInt64
				o.PartSizeBytes = int64(opts.BufferSize)
				o.Concurrency = opts.MaxConcurrency
			})
			if opts.BeforeWrite != nil {
				if err := opts.BeforeWrite(as); err != nil {
					return err
				}
			}
			return sha256Upload(as, nil)
		},
	})
}

func (b *s3Bucket) Copy(ctx context.Context, dst, src string, opts *driver.CopyOptions) error {
	return b.Bucket.Copy(ctx, dst, src, &blob.CopyOptions{BeforeCopy: opts.BeforeCopy})
}

func (b *s3Bucket) SignedURL(ctx context.Context, key string, opts *driver.SignedURLOptions) (string, error) {
	return b.Bucket.SignedURL(ctx, key, &blob.SignedURLOptions{Expiry: opts.Expiry, Method: opts.Method,
		ContentType: opts.ContentType, EnforceAbsentContentType: opts.EnforceAbsentContentType, BeforeSign: opts.BeforeSign})
}

// sha256Upload selects S3's SHA-256 upload checksum through its documented escape hatch.
// File stores do not expose this S3 input and retain their ordinary write path.
func sha256Upload(as func(any) bool, checksum *string) error {
	var input *transfermanager.UploadObjectInput
	if as(&input) {
		input.ChecksumAlgorithm = types.ChecksumAlgorithmSha256
		if checksum != nil {
			input.ChecksumSHA256 = checksum
		}
	}
	return nil
}
