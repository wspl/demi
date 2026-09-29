package storage

import (
	"crypto/sha256"
	"encoding/base64"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/service/s3"
)

// Cost: one local TLS server and seven requests, no AWS service or credentials.
func TestS3ConditionalChecksumMetadataAndPresigning(t *testing.T) {
	payload := []byte("fixture object")
	sum := sha256.Sum256(payload)
	checksum := base64.StdEncoding.EncodeToString(sum[:])
	puts := 0
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("list-type") == "2" {
			w.Header().Set("Content-Type", "application/xml")
			if r.URL.Query().Get("continuation-token") == "" {
				fmt.Fprint(w, `<ListBucketResult><IsTruncated>true</IsTruncated><NextContinuationToken>next</NextContinuationToken><Contents><Key>blobs/user/one</Key><LastModified>2023-11-14T22:13:20Z</LastModified><Size>14</Size></Contents></ListBucketResult>`)
			} else {
				fmt.Fprint(w, `<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>blobs/user/two</Key><LastModified>2023-11-14T22:13:20Z</LastModified><Size>14</Size></Contents></ListBucketResult>`)
			}
			return
		}
		if r.URL.Path != "/fixture-bucket/blobs/user/object" {
			t.Errorf("unexpected path %s", r.URL.Path)
		}
		switch r.Method {
		case "HEAD":
			w.WriteHeader(http.StatusNotFound)
		case "PUT":
			puts++
			if r.Header.Get("If-None-Match") != "*" || r.Header.Get("X-Amz-Checksum-Sha256") != checksum || r.Header.Get("X-Amz-Meta-Label") != "fixture" {
				t.Error("conditional write lost checksum or metadata")
			}
			body, err := io.ReadAll(r.Body)
			if err != nil || string(body) != string(payload) {
				t.Errorf("body: %q %v", body, err)
			}
			if puts > 1 {
				w.Header().Set("Content-Type", "application/xml")
				w.WriteHeader(412)
				fmt.Fprint(w, `<Error><Code>PreconditionFailed</Code></Error>`)
			}
		case "GET":
			w.Header().Set("X-Amz-Checksum-Sha256", checksum)
			w.Write(payload)
		case "DELETE":
			w.WriteHeader(204)
		default:
			t.Errorf("method %s", r.Method)
		}
	}))
	defer server.Close()
	client := s3.NewFromConfig(aws.Config{Region: "us-east-1", Credentials: aws.NewCredentialsCache(credentials.NewStaticCredentialsProvider("fixture-id", "fixture-secret", "")), HTTPClient: server.Client()}, func(o *s3.Options) { o.BaseEndpoint = aws.String(server.URL); o.UsePathStyle = true })
	objects := &S3Objects{client: client, presign: s3.NewPresignClient(client), bucket: "fixture-bucket"}
	key := "blobs/user/object"
	ctx := t.Context()
	if _, err := objects.Head(ctx, key); err != ErrObjectNotFound {
		t.Fatalf("missing HEAD: %v", err)
	}
	if err := objects.Create(ctx, key, payload, map[string]string{"label": "fixture"}); err != nil {
		t.Fatal(err)
	}
	if err := objects.Create(ctx, key, payload, map[string]string{"label": "fixture"}); err != ErrObjectExists {
		t.Fatalf("conditional conflict: %v", err)
	}
	data, err := objects.Get(ctx, key)
	if err != nil || string(data) != string(payload) {
		t.Fatalf("GET: %q %v", data, err)
	}
	listed, err := objects.List(ctx, "blobs/user/")
	if err != nil || len(listed) != 2 {
		t.Fatalf("pagination: %+v %v", listed, err)
	}
	signed, err := objects.PresignGet(ctx, key, 5*time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	parsed, err := url.Parse(signed)
	if err != nil {
		t.Fatal(err)
	}
	if parsed.Query().Get("X-Amz-Expires") != "300" || !strings.Contains(parsed.Query().Get("X-Amz-Credential"), "fixture-id/") || parsed.Query().Get("X-Amz-Signature") == "" {
		t.Fatal("GET was not signed with its lifetime")
	}
	if err = objects.Delete(ctx, key); err != nil {
		t.Fatal(err)
	}
}
