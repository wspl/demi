package blobs_test

import (
	"bytes"
	"context"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"encoding/base64"
	"io"
	"net/http"
	"net/http/httptest"
	"net/http/httputil"
	"net/url"
	"os"
	"path/filepath"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/aws/aws-sdk-go-v2/service/s3"
	"gocloud.dev/blob"
	"gocloud.dev/gcerrors"

	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
	"github.com/wspl/demi/internal/core"
)

// Verify the real configuration path, transport ownership, checksum policy and
// publication metadata over loopback TLS. No request can reach a vendor.
func TestConfiguredS3ChecksumsAndPublication(t *testing.T) {
	ctx := t.Context()
	fake, err := blobstest.StartS3(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := fake.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	endpoint, err := url.Parse(fake.Endpoint)
	if err != nil {
		t.Fatal(err)
	}
	proxy := httputil.NewSingleHostReverseProxy(endpoint)
	var mu sync.Mutex
	var checksums []string
	var requests atomic.Int64
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests.Add(1)
		if r.Method == http.MethodPut {
			mu.Lock()
			checksum := r.Header.Get("X-Amz-Checksum-Sha256")
			if checksum == "" {
				checksum = r.Header.Get("X-Amz-Trailer")
			}
			checksums = append(checksums, checksum)
			mu.Unlock()
		}
		proxy.ServeHTTP(w, r)
	}))
	defer server.Close()
	t.Setenv("AWS_ACCESS_KEY_ID", "fake")
	t.Setenv("AWS_SECRET_ACCESS_KEY", "fake")
	t.Setenv("AWS_SESSION_TOKEN", "")
	t.Setenv("AWS_WEB_IDENTITY_TOKEN_FILE", "")
	t.Setenv("AWS_EC2_METADATA_DISABLED", "true")
	t.Setenv("AWS_CA_BUNDLE", "")
	invalidProfile := filepath.Join(t.TempDir(), "credentials")
	if err := os.WriteFile(invalidProfile, []byte("not an ini file ["), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("AWS_SHARED_CREDENTIALS_FILE", invalidProfile)
	t.Setenv("AWS_CONFIG_FILE", invalidProfile)
	t.Setenv("AWS_PROFILE", "must-not-be-read")
	config := blobs.S3Config{Bucket: "demi", Region: "us-east-1", Endpoint: &server.URL, ForcePathStyle: true}
	bucket, err := config.Open(ctx)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := bucket.Close(); err != nil {
			t.Error(err)
		}
	})
	// Install the test server's trust root through the documented driver escape
	// hatch before any request. Production still validates HTTPS certificates.
	var service *s3.Client
	if !bucket.As(&service) {
		t.Fatal("S3 client unavailable")
	}
	client, ok := service.Options().HTTPClient.(*http.Client)
	if !ok {
		t.Fatal("S3 client does not expose its owned HTTP client")
	}
	transport, ok := client.Transport.(*http.Transport)
	if !ok {
		t.Fatal("S3 client does not expose its owned transport")
	}
	roots := x509.NewCertPool()
	roots.AddCert(server.Certificate())
	transport.TLSClientConfig = &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12}
	data := []byte("published archive")
	options := &blob.WriterOptions{
		IfNotExist:      true,
		ContentType:     "application/octet-stream",
		ContentEncoding: "gzip",
		Metadata:        map[string]string{"sha256": string(core.BlobRefOf(data))},
	}
	if err := bucket.WriteAll(ctx, "artifacts/archive", data, options); err != nil {
		t.Fatal(err)
	}
	if err := bucket.WriteAll(
		ctx,
		"artifacts/archive",
		[]byte("different"),
		options,
	); gcerrors.Code(
		err,
	) != gcerrors.FailedPrecondition {
		t.Fatalf("conditional write = %v", err)
	}
	attrs, err := bucket.Attributes(ctx, "artifacts/archive")
	if err != nil {
		t.Fatal(err)
	}
	if attrs.ContentEncoding != "gzip" || attrs.Metadata["sha256"] != string(core.BlobRefOf(data)) {
		t.Fatalf("attributes = %+v", attrs)
	}
	reader, err := bucket.NewRangeReader(ctx, "artifacts/archive", 1, 4, nil)
	if err != nil {
		t.Fatal(err)
	}
	got, readErr := io.ReadAll(reader)
	closeErr := reader.Close()
	if readErr != nil || closeErr != nil || !bytes.Equal(got, data[1:5]) {
		t.Fatalf("range = %q, %v, %v", got, readErr, closeErr)
	}
	signed, err := bucket.SignedURL(ctx, "artifacts/archive", nil)
	if err != nil {
		t.Fatal(err)
	}
	if parsed, err := url.Parse(signed); err != nil || parsed.Query().Get("X-Amz-Signature") == "" {
		t.Fatalf("signed URL = %s, %v", signed, err)
	}
	mu.Lock()
	observed := append([]string(nil), checksums...)
	mu.Unlock()
	sum := sha256.Sum256(data)
	if len(observed) != 2 ||
		(observed[0] != base64.StdEncoding.EncodeToString(sum[:]) && observed[0] != "x-amz-checksum-sha256") {
		t.Fatalf("checksums = %q", observed)
	}
	before := requests.Load()
	large := bytes.Repeat([]byte("x"), 25<<20)
	namespace := blobs.New(bucket, core.SystemClock{}).ForUser("ana")
	ref, err := namespace.Put(ctx, large)
	if err != nil {
		t.Fatal(err)
	}
	if requests.Load()-before != 2 {
		t.Fatalf("large blob made %d requests, want HEAD and PUT", requests.Load()-before)
	}
	mu.Lock()
	largeChecksum := checksums[len(checksums)-1]
	mu.Unlock()
	largeSum := sha256.Sum256(large)
	if largeChecksum != base64.StdEncoding.EncodeToString(largeSum[:]) {
		t.Fatal("blob PUT did not carry its SHA-256 checksum header")
	}

	if got, exists := fake.Object("blobs/ana/" + string(ref)); !exists || !bytes.Equal(got, large) {
		t.Fatal("large blob was not published intact")
	}
}

// The fake supports the raw Go Cloud client's multipart path as well as single puts.
func TestS3FakeMultipartMedia(t *testing.T) {
	ctx := t.Context()
	fake, err := blobstest.StartS3(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := fake.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	bucket, err := fake.Client(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := bucket.Close(); err != nil {
			t.Error(err)
		}
	}()
	namespace := blobs.New(bucket, core.SystemClock{}).ForUser("ana")
	data := bytes.Repeat([]byte("x"), 25<<20)
	ref, err := namespace.Put(ctx, data)
	if err != nil {
		t.Fatal(err)
	}
	got, found, err := namespace.Read(ctx, ref)
	if len(fake.Written()) != 1 {
		t.Fatalf("large upload writes = %v", fake.Written())
	}

	if err != nil || !found || !bytes.Equal(got, data) {
		t.Fatalf("large blob: length=%d, found=%t, error=%v", len(got), found, err)
	}
}

// onePerPage forces actual S3 pagination without writing a thousand objects.
type onePerPage struct{ blobs.Objects }

func (p onePerPage) List(options *blob.ListOptions) *blob.ListIterator {
	options.BeforeList = func(as func(any) bool) error {
		var request *s3.ListObjectsV2Input
		if as(&request) {
			one := int32(1)
			request.MaxKeys = &one
		}
		return nil
	}
	return p.Objects.List(options)
}

func TestS3ListingReadsAllPages(t *testing.T) {
	ctx := t.Context()
	fake, err := blobstest.StartS3(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := fake.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	bucket, err := fake.Client(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := bucket.Close(); err != nil {
			t.Error(err)
		}
	}()
	namespace := blobs.New(onePerPage{bucket}, core.SystemClock{}).ForUser("ana")
	want := make(map[core.BlobRef]bool)
	for _, data := range []string{"first", "second", "third"} {
		ref, err := namespace.Put(ctx, core.B64Bytes(data))
		if err != nil {
			t.Fatal(err)
		}
		want[ref] = true
	}
	listing, err := namespace.List(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(listing) != len(want) {
		t.Fatalf("listing length = %d, want %d", len(listing), len(want))
	}
	for _, found := range listing {
		if !want[found.Blob] {
			t.Fatalf("unexpected blob %s", found.Blob)
		}
		delete(want, found.Blob)
	}
}
