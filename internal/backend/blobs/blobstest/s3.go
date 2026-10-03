package blobstest

import (
	"bufio"
	"bytes"
	"context"
	"crypto/md5" // S3's ETag for a single-part object.
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/xml"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/http/httputil"
	"net/textproto"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	awshttp "github.com/aws/aws-sdk-go-v2/aws/transport/http"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"gocloud.dev/blob"
	"gocloud.dev/blob/s3blob"

	"github.com/wspl/demi/internal/backend/blobs"
)

// FakeS3 is an S3-compatible service in memory. It handles puts, conditional
// creation, reads with metadata, and deletions without checking signatures.
// Its owner must close it after its clients stop using it.
type FakeS3 struct {
	// Endpoint is the local HTTP URL at which the fake accepts requests.
	Endpoint string
	server   *httptest.Server
	// mu protects objects, written, uploads and clients; no IO runs under it.
	mu         sync.Mutex
	objects    map[string]s3Object
	written    []string
	uploads    map[string]*multipartUpload
	nextUpload uint64
	clients    []*http.Client
}

type s3Object struct {
	data    []byte
	headers http.Header
	written time.Time
}
type multipartUpload struct {
	key     string
	headers http.Header
	parts   map[int][]byte
}

// StartS3 starts a fake on a loopback port allocated by the operating system.
// The caller owns it and must call Close, which stops and joins its server.
func StartS3(ctx context.Context) (*FakeS3, error) {
	var lc net.ListenConfig
	listener, err := lc.Listen(ctx, "tcp", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	f := &FakeS3{objects: make(map[string]s3Object), uploads: make(map[string]*multipartUpload)}
	f.server = &httptest.Server{Listener: listener, Config: &http.Server{Handler: http.HandlerFunc(f.serve)}}
	f.server.Start()
	f.Endpoint = f.server.URL
	return f, nil
}

// Close stops and joins the fake's server and releases its listener and connections.
func (f *FakeS3) Close(_ context.Context) error {
	// httptest.Close closes connections and joins requests, including abandoned ones.
	f.server.Close()
	f.mu.Lock()
	clients := f.clients
	f.clients = nil
	f.mu.Unlock()
	for _, client := range clients {
		client.CloseIdleConnections()
	}
	return nil
}

// Config returns the fake's bucket as the backend's configuration names it.
// Its HTTP endpoint is test-only and does not pass production HTTPS validation.
func (f *FakeS3) Config() blobs.S3Config {
	endpoint := f.Endpoint
	return blobs.S3Config{Bucket: "demi", Region: "us-east-1", Endpoint: &endpoint, ForcePathStyle: true}
}

// Client opens the backend's client of the fake, with fake credentials and
// plain HTTP permitted. The caller must close the returned bucket.
func (f *FakeS3) Client(ctx context.Context) (*blob.Bucket, error) {
	client := &http.Client{Transport: awshttp.NewBuildableClient().GetTransport()}
	config := f.Config()
	service := s3.New(s3.Options{
		Region:       config.Region,
		BaseEndpoint: config.Endpoint,
		UsePathStyle: true,
		Credentials:  credentials.NewStaticCredentialsProvider("fake", "fake", ""),
		HTTPClient:   client,
	})
	bucket, err := s3blob.OpenBucket(ctx, service, config.Bucket, nil)
	if err != nil {
		client.CloseIdleConnections()
		return nil, err
	}
	f.mu.Lock()
	f.clients = append(f.clients, client)
	f.mu.Unlock()
	return bucket, nil
}

// Written returns the keys of objects put, in the order they were put.
func (f *FakeS3) Written() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]string{}, f.written...)
}

// Object returns a copy of the bytes at key and whether the object exists.
func (f *FakeS3) Object(key string) ([]byte, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	object, found := f.objects[key]
	return bytes.Clone(object.data), found
}

// serve implements the S3 requests made by the real Go Cloud client.
func (f *FakeS3) serve(w http.ResponseWriter, r *http.Request) {
	_, key, _ := strings.Cut(strings.TrimPrefix(r.URL.Path, "/"), "/")
	if r.Method == http.MethodGet && r.URL.Query().Get("list-type") == "2" {
		f.list(w, r)
		return
	}
	if r.URL.Query().Has("uploads") || r.URL.Query().Has("uploadId") {
		f.multipart(w, r, key)
		return
	}
	switch r.Method {
	case http.MethodPut:
		data, err := readS3Body(r)
		if err != nil {
			s3Failure(w, http.StatusBadRequest, "IncompleteBody")
			return
		}
		if !validChecksum(data, r.Header.Get("x-amz-checksum-sha256")) {
			s3Failure(w, http.StatusBadRequest, "BadDigest")
			return
		}
		if !f.put(key, data, r.Header, r.Header.Get("If-None-Match") == "*") {
			s3Failure(w, http.StatusPreconditionFailed, "PreconditionFailed")
			return
		}
		w.Header().Set("ETag", objectETag(data))
		w.WriteHeader(http.StatusOK)
	case http.MethodHead, http.MethodGet:
		f.readObject(w, r, key)
	case http.MethodDelete:
		f.mu.Lock()
		delete(f.objects, key)
		f.mu.Unlock()
		w.WriteHeader(http.StatusNoContent)
	default:
		s3Failure(w, http.StatusMethodNotAllowed, "MethodNotAllowed")
	}
}

// put atomically enforces S3 conditional creation and records successful writes.
func (f *FakeS3) put(key string, data []byte, headers http.Header, conditional bool) bool {
	metadata := make(http.Header)
	for name, values := range headers {
		lower := strings.ToLower(name)
		if strings.HasPrefix(lower, "x-amz-meta-") || lower == "content-encoding" || lower == "content-type" ||
			lower == "cache-control" ||
			lower == "content-disposition" ||
			lower == "content-language" {
			metadata[name] = append([]string(nil), values...)
		}
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if _, found := f.objects[key]; found && conditional {
		return false
	}
	f.objects[key] = s3Object{data: data, headers: metadata, written: time.Now().UTC()}
	f.written = append(f.written, key)
	return true
}

// validChecksum validates an S3 upload checksum when its client supplies one.
func validChecksum(data []byte, checksum string) bool {
	if checksum == "" {
		return true
	}
	sum := sha256.Sum256(data)
	return checksum == base64.StdEncoding.EncodeToString(sum[:])
}

// objectETag supplies the S3 ETag consumed by the SDK's metadata decoder.
func objectETag(data []byte) string {
	sum := md5.Sum(data)
	return `"` + hex.EncodeToString(sum[:]) + `"`
}

// s3Failure writes the S3 error envelope understood by the SDK.
func s3Failure(w http.ResponseWriter, status int, code string) {
	w.Header().Set("Content-Type", "application/xml")
	w.WriteHeader(status)
	// A disconnected test client needs no further response.
	_ = xml.NewEncoder(w).Encode(struct {
		XMLName xml.Name `xml:"Error"`
		Code    string
		Message string
	}{Code: code, Message: code})
}

// list serves sorted and paginated S3 namespace listings.
func (f *FakeS3) list(w http.ResponseWriter, r *http.Request) {
	query := r.URL.Query()
	prefix, after := query.Get("prefix"), query.Get("continuation-token")
	maxKeys := 1000
	if value := query.Get("max-keys"); value != "" {
		n, err := strconv.Atoi(value)
		if err != nil || n < 0 {
			s3Failure(w, http.StatusBadRequest, "InvalidArgument")
			return
		}
		maxKeys = n
	}
	type item struct {
		Key          string
		LastModified string
		ETag         string
		Size         int
	}
	result := struct {
		XMLName               xml.Name `xml:"ListBucketResult"`
		Contents              []item
		IsTruncated           bool
		NextContinuationToken string `xml:",omitempty"`
	}{}
	f.mu.Lock()
	keys := make([]string, 0, len(f.objects))
	for key := range f.objects {
		if strings.HasPrefix(key, prefix) && key > after {
			keys = append(keys, key)
		}
	}
	sort.Strings(keys)
	for i, key := range keys {
		if i == maxKeys {
			result.IsTruncated = true
			break
		}
		object := f.objects[key]
		result.Contents = append(
			result.Contents,
			item{
				Key:          key,
				LastModified: object.written.Format(time.RFC3339Nano),
				ETag:         objectETag(object.data),
				Size:         len(object.data),
			},
		)
	}
	if result.IsTruncated && len(result.Contents) > 0 {
		result.NextContinuationToken = result.Contents[len(result.Contents)-1].Key
	}
	f.mu.Unlock()
	w.Header().Set("Content-Type", "application/xml")
	// A disconnected test client needs no further response.
	_ = xml.NewEncoder(w).Encode(result)
}

// multipart supports the SDK's multipart path for uploads larger than one part.
func (f *FakeS3) multipart(w http.ResponseWriter, r *http.Request, key string) {
	query := r.URL.Query()
	if query.Has("uploads") && r.Method == http.MethodPost {
		f.initiateMultipart(w, r, key)
		return
	}
	id := query.Get("uploadId")
	if r.Method == http.MethodDelete {
		f.mu.Lock()
		delete(f.uploads, id)
		f.mu.Unlock()
		w.WriteHeader(http.StatusNoContent)
		return
	}
	data, err := readS3Body(r)
	if err != nil {
		s3Failure(w, http.StatusBadRequest, "IncompleteBody")
		return
	}
	f.mu.Lock()
	upload := f.uploads[id]
	f.mu.Unlock()
	if upload == nil || upload.key != key {
		s3Failure(w, http.StatusNotFound, "NoSuchUpload")
		return
	}
	if r.Method == http.MethodPut {
		part, err := strconv.Atoi(query.Get("partNumber"))
		if err != nil || part < 1 || !validChecksum(data, r.Header.Get("x-amz-checksum-sha256")) {
			s3Failure(w, http.StatusBadRequest, "InvalidPart")
			return
		}
		f.mu.Lock()
		upload.parts[part] = data
		f.mu.Unlock()
		sum := sha256.Sum256(data)
		w.Header().Set("x-amz-checksum-sha256", base64.StdEncoding.EncodeToString(sum[:]))
		w.Header().Set("ETag", objectETag(data))
		return
	}
	if r.Method != http.MethodPost {
		s3Failure(w, http.StatusMethodNotAllowed, "MethodNotAllowed")
		return
	}
	f.completeMultipart(w, r, key, id, upload, data)
}

// readS3Body decodes the SDK's S3 chunked checksum framing over HTTPS.
func readS3Body(r *http.Request) ([]byte, error) {
	encodings := strings.Split(strings.Join(r.Header.Values("Content-Encoding"), ","), ",")
	chunked := false
	kept := encodings[:0]
	for _, encoding := range encodings {
		if strings.TrimSpace(encoding) == "aws-chunked" {
			chunked = true
		} else {
			kept = append(kept, encoding)
		}
	}
	if !chunked {
		return io.ReadAll(r.Body)
	}
	reader := bufio.NewReader(r.Body)
	data, err := io.ReadAll(httputil.NewChunkedReader(reader))
	if err != nil {
		return nil, err
	}
	trailers, err := textproto.NewReader(reader).ReadMIMEHeader()
	if err != nil {
		return nil, err
	}
	for name, values := range trailers {
		r.Header[name] = values
	}
	r.Header.Set("Content-Encoding", strings.Join(kept, ","))
	return data, nil
}

func (f *FakeS3) readObject(w http.ResponseWriter, r *http.Request, key string) {
	f.mu.Lock()
	object, found := f.objects[key]
	f.mu.Unlock()
	if !found {
		s3Failure(w, http.StatusNotFound, "NoSuchKey")
		return
	}
	for name, values := range object.headers {
		w.Header()[name] = append([]string(nil), values...)
	}
	w.Header().Set("ETag", objectETag(object.data))
	w.Header().Set("Last-Modified", object.written.UTC().Format(http.TimeFormat))
	w.Header().Set("Content-Length", strconv.Itoa(len(object.data)))
	if r.Method == http.MethodHead {
		w.WriteHeader(http.StatusOK)
		return
	}
	http.ServeContent(w, r, key, object.written, bytes.NewReader(object.data))
}

func (f *FakeS3) initiateMultipart(w http.ResponseWriter, r *http.Request, key string) {
	f.mu.Lock()
	f.nextUpload++
	id := strconv.FormatUint(f.nextUpload, 10)
	f.uploads[id] = &multipartUpload{key: key, headers: r.Header.Clone(), parts: make(map[int][]byte)}
	f.mu.Unlock()
	// A disconnected test client needs no further response.
	_ = xml.NewEncoder(w).Encode(struct {
		XMLName  xml.Name `xml:"InitiateMultipartUploadResult"`
		UploadID string   `xml:"UploadId"`
	}{UploadID: id})
}

func (f *FakeS3) completeMultipart(
	w http.ResponseWriter,
	r *http.Request,
	key, id string,
	upload *multipartUpload,
	data []byte,
) {
	var completion struct {
		Parts []struct {
			Number int `xml:"PartNumber"`
			ETag   string
		} `xml:"Part"`
	}
	if err := xml.Unmarshal(data, &completion); err != nil {
		s3Failure(w, http.StatusBadRequest, "MalformedXML")
		return
	}
	var joined []byte
	valid := true
	f.mu.Lock()
	for _, part := range completion.Parts {
		value, found := upload.parts[part.Number]
		if !found || objectETag(value) != part.ETag {
			valid = false
			break
		}
		joined = append(joined, value...)
	}
	delete(f.uploads, id)
	f.mu.Unlock()
	if !valid {
		s3Failure(w, http.StatusBadRequest, "InvalidPart")
		return
	}
	if !f.put(key, joined, upload.headers, r.Header.Get("If-None-Match") == "*") {
		s3Failure(w, http.StatusPreconditionFailed, "PreconditionFailed")
		return
	}
	// A disconnected test client needs no further response.
	_ = xml.NewEncoder(w).Encode(struct {
		XMLName xml.Name `xml:"CompleteMultipartUploadResult"`
		ETag    string
	}{ETag: objectETag(joined)})
}
