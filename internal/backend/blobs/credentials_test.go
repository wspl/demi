package blobs_test

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/aws/aws-sdk-go-v2/service/s3"
	"github.com/wspl/demi/internal/backend/blobs"
)

type credentialTransport func(*http.Request) (*http.Response, error)

func (f credentialTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	return f(r)
}

// Real SDK credential providers are exercised with an entirely scripted transport;
// unexpected requests fail locally, so the tests cannot contact cloud metadata.
func TestCredentialSourcesExcludeProfiles(t *testing.T) {
	for _, source := range []string{"environment", "web identity", "container", "instance", "incomplete environment"} {
		t.Run(source, func(t *testing.T) {
			for _, name := range []string{
				"AWS_ACCESS_KEY_ID",
				"AWS_SECRET_ACCESS_KEY",
				"AWS_ACCESS_KEY",
				"AWS_SECRET_KEY",
				"AWS_SESSION_TOKEN",
				"AWS_WEB_IDENTITY_TOKEN_FILE",
				"AWS_ROLE_ARN",
				"AWS_ROLE_SESSION_NAME",
				"AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
				"AWS_CONTAINER_CREDENTIALS_FULL_URI",
				"AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE",
			} {
				t.Setenv(name, "")
			}
			t.Setenv("AWS_EC2_METADATA_DISABLED", "false")
			t.Setenv("AWS_EC2_METADATA_SERVICE_ENDPOINT", "http://metadata.invalid")
			t.Setenv("AWS_PROFILE", "ignored-profile")
			tokenFile := filepath.Join(t.TempDir(), "token")
			if err := os.WriteFile(tokenFile, []byte("test-token"), 0o600); err != nil {
				t.Fatal(err)
			}
			switch source {
			case "environment":
				t.Setenv("AWS_ACCESS_KEY_ID", "test-key")
				t.Setenv("AWS_SECRET_ACCESS_KEY", "test-secret")
			case "incomplete environment":
				t.Setenv("AWS_ACCESS_KEY_ID", "test-key")
			case "web identity":
				t.Setenv("AWS_WEB_IDENTITY_TOKEN_FILE", tokenFile)
				t.Setenv("AWS_ROLE_ARN", "arn:aws:iam::123456789012:role/test")
			case "container":
				t.Setenv("AWS_CONTAINER_CREDENTIALS_FULL_URI", "http://container.invalid/credentials")
				t.Setenv("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE", tokenFile)
			}
			endpoint := "https://objects.invalid"
			bucket, err := (blobs.S3Config{Bucket: "demi", Region: "us-east-1", Endpoint: &endpoint}).Open(t.Context())
			if source == "incomplete environment" {
				if err == nil {
					if closeErr := bucket.Close(); closeErr != nil {
						t.Error(closeErr)
					}
					t.Fatal("incomplete credentials accepted")
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := bucket.Close(); err != nil {
					t.Error(err)
				}
			}()
			var service *s3.Client
			if !bucket.As(&service) {
				t.Fatal("S3 client unavailable")
			}
			client, ok := service.Options().HTTPClient.(*http.Client)
			if !ok {
				t.Fatal("owned HTTP client unavailable")
			}
			client.Transport = credentialTransport(func(r *http.Request) (*http.Response, error) {
				var body string
				switch {
				case source == "web identity" && r.URL.Host == "sts.us-east-1.amazonaws.com":
					if err := r.ParseForm(); err != nil {
						return nil, err
					}
					if r.Form.Get("WebIdentityToken") != "test-token" ||
						r.Form.Get("RoleSessionName") != "WebIdentitySession" {
						return nil, fmt.Errorf("unexpected web identity form")
					}
					body = `<AssumeRoleWithWebIdentityResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">` +
						`<AssumeRoleWithWebIdentityResult><Credentials><AccessKeyId>test-key</AccessKeyId>` +
						`<SecretAccessKey>test-secret</SecretAccessKey><SessionToken>test-session</SessionToken>` +
						`<Expiration>2099-01-01T00:00:00Z</Expiration></Credentials>` +
						`</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>`
				case source == "container" && r.URL.Host == "container.invalid":
					if r.Header.Get("Authorization") != "test-token" {
						return nil, fmt.Errorf("missing container authorization")
					}
					body = `{"AccessKeyId":"test-key","SecretAccessKey":"test-secret","Token":"test-session",` +
						`"Expiration":"2099-01-01T00:00:00Z"}`
				case source == "instance" && r.URL.Host == "metadata.invalid":
					switch r.URL.Path {
					case "/latest/api/token":
						body = "imds-token"
					case "/latest/meta-data/iam/security-credentials/":
						body = "test-role"
					case "/latest/meta-data/iam/security-credentials/test-role":
						body = `{"Code":"Success","AccessKeyId":"test-key","SecretAccessKey":"test-secret",` +
							`"Token":"test-session","Expiration":"2099-01-01T00:00:00Z"}`
					default:
						return nil, fmt.Errorf("unexpected metadata path %s", r.URL.Path)
					}
				default:
					return nil, fmt.Errorf("unexpected credential request to %s", r.URL.Host)
				}
				return &http.Response{
					StatusCode: http.StatusOK,
					Header:     make(http.Header),
					Body:       io.NopCloser(strings.NewReader(body)),
					Request:    r,
				}, nil
			})
			value, err := service.Options().Credentials.Retrieve(t.Context())
			if err != nil {
				t.Fatal(err)
			}
			if value.AccessKeyID != "test-key" || value.SecretAccessKey != "test-secret" {
				t.Fatal("incorrect credential source")
			}
		})
	}
}

func TestBucketCloseJoinsCanceledCredentialRefresh(t *testing.T) {
	for _, name := range []string{
		"AWS_ACCESS_KEY_ID",
		"AWS_SECRET_ACCESS_KEY",
		"AWS_WEB_IDENTITY_TOKEN_FILE",
		"AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
	} {
		t.Setenv(name, "")
	}
	token := filepath.Join(t.TempDir(), "token")
	if err := os.WriteFile(token, []byte("test-token"), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("AWS_CONTAINER_CREDENTIALS_FULL_URI", "http://container.invalid/credentials")
	t.Setenv("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE", token)
	endpoint := "https://objects.invalid"
	bucket, err := (blobs.S3Config{Bucket: "demi", Region: "us-east-1", Endpoint: &endpoint}).Open(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if bucket != nil {
			if err := bucket.Close(); err != nil {
				t.Error(err)
			}
		}
	})
	var service *s3.Client
	if !bucket.As(&service) {
		t.Fatal("S3 client unavailable")
	}
	client, ok := service.Options().HTTPClient.(*http.Client)
	if !ok {
		t.Fatal("owned HTTP client unavailable")
	}
	reached, finished, release := make(chan struct{}), make(chan struct{}), make(chan struct{})
	client.Transport = credentialTransport(func(r *http.Request) (*http.Response, error) {
		close(reached)
		defer close(finished)
		select {
		case <-r.Context().Done():
			return nil, r.Context().Err()
		case <-release:
			return nil, context.Canceled
		}
	})
	ctx, cancel := context.WithCancel(t.Context())
	var workers sync.WaitGroup
	defer func() {
		cancel()
		close(release)
		workers.Wait()
	}()
	result := make(chan error, 1)
	workers.Go(func() {
		_, err := service.Options().Credentials.Retrieve(ctx)
		result <- err
	})
	<-reached
	cancel()
	if err := <-result; !errors.Is(err, context.Canceled) {
		t.Fatalf("canceled credential request = %v", err)
	}
	if err := bucket.Close(); err != nil {
		t.Fatal(err)
	}
	bucket = nil
	select {
	case <-finished:
	default:
		t.Fatal("bucket close left credential IO running")
	}
}
