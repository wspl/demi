package runners

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/netip"
	"net/url"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/backend/blobs"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
	"github.com/wspl/demi/internal/commandproto"
	"gocloud.dev/blob"
)

// nativeFixture writes test-only executables and their generated descriptor.
func nativeFixture(t *testing.T, targets []string) (NativeRelease, commandproto.PackageDescriptor) {
	t.Helper()
	directory := t.TempDir()
	descriptor := commandproto.PackageDescriptor{
		ID:              "example.commands",
		Version:         "1.0.0+build",
		ProtocolVersion: 1,
		Operations:      []string{"fixture"},
		Targets:         make(map[string]commandproto.PackageArtifact),
	}
	for _, target := range targets {
		data := []byte("test-only artifact " + target)
		suffix := ""
		if strings.Contains(target, "windows") {
			suffix = ".exe"
		}
		if err := os.MkdirAll(filepath.Join(directory, target), 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(directory, target, "commands"+suffix), data, 0o600); err != nil {
			t.Fatal(err)
		}
		descriptor.Targets[target] = commandproto.PackageArtifact{
			SHA256: fmt.Sprintf("%x", sha256.Sum256(data)),
			Size:   uint64(len(data)),
		}
	}
	writeDescriptor(t, directory, descriptor)
	return NativeRelease{Directory: directory, Executable: "commands"}, descriptor
}

// writeDescriptor publishes the fixture's one authoritative package contract.
func writeDescriptor(t *testing.T, directory string, descriptor commandproto.PackageDescriptor) {
	t.Helper()
	data, err := descriptor.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, "descriptor.json"), data, 0o600); err != nil {
		t.Fatal(err)
	}
}

// publicationBucket owns a loopback S3 fixture and its publication client.
func publicationBucket(t *testing.T) (*blobstest.FakeS3, *blob.Bucket) {
	t.Helper()
	fake, err := blobstest.StartS3(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := fake.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	bucket, err := fake.Client(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := bucket.Close(); err != nil {
			t.Error(err)
		}
	})
	return fake, bucket
}

func TestPublicationOrderAndImmutableVersion(t *testing.T) {
	fake, bucket := publicationBucket(t)
	release, descriptor := nativeFixture(t, commandproto.Targets)
	catalog, err := publish(t.Context(), []NativeRelease{release}, "native", bucket)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := catalog.Package("example.commands"); !ok {
		t.Fatal("package unavailable after publication")
	}
	written := fake.Written()
	if len(written) != 8 {
		t.Fatalf("writes: %v", written)
	}
	for _, key := range written[:6] {
		if !strings.HasPrefix(key, "native/blobs/") {
			t.Fatalf("descriptor before artifacts: %v", written)
		}
	}
	const claim = "native/packages/example.commands/1.0.0%2Bbuild.json"
	if !strings.HasPrefix(written[6], "native/descriptors/") || written[7] != claim {
		t.Fatalf("publication order: %v", written)
	}
	artifact := descriptor.Targets[commandproto.Targets[0]]
	key := "native/blobs/" + artifact.SHA256
	encoded, ok := fake.Object(key)
	if !ok {
		t.Fatal("missing executable")
	}
	plain, err := os.ReadFile(filepath.Join(release.Directory, commandproto.Targets[0], "commands"))
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(encoded, plain) {
		t.Fatal("executable was not encoded")
	}
	client := artifacts.NewClientAllowingHTTP()
	defer client.Close()
	var downloaded bytes.Buffer
	if err := artifacts.Download(
		t.Context(),
		client,
		fake.Endpoint+"/demi/"+key,
		artifacts.Digest{SHA256: artifact.SHA256, Size: artifact.Size},
		&downloaded,
	); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(downloaded.Bytes(), plain) {
		t.Fatal("download differs from declared executable")
	}
	if _, err := publish(t.Context(), []NativeRelease{release}, "native", bucket); err != nil {
		t.Fatal(err)
	}
	if len(fake.Written()) != 8 {
		t.Fatal("restart overwrote objects")
	}
	descriptor.Operations = append(descriptor.Operations, "changed")
	writeDescriptor(t, release.Directory, descriptor)
	_, err = publish(t.Context(), []NativeRelease{release}, "native", bucket)
	if !errors.Is(err, ErrArtifactConflict) {
		t.Fatalf("version conflict: %v", err)
	}
	original, ok := fake.Object(claim)
	if !ok {
		t.Fatal("version mapping vanished")
	}
	kept, err := commandproto.DecodePackageDescriptor(original)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(kept.Operations, []string{"fixture"}) {
		t.Fatal("immutable mapping changed")
	}
}

func TestInvalidReleasePublishesNothing(t *testing.T) {
	fake, bucket := publicationBucket(t)
	release, descriptor := nativeFixture(t, commandproto.Targets)
	path := filepath.Join(release.Directory, commandproto.Targets[5], "commands.exe")
	original, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("corrupt"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, err = publish(t.Context(), []NativeRelease{release}, "native", bucket)
	if err == nil || !strings.Contains(err.Error(), "does not match the descriptor") {
		t.Fatalf("corrupt artifact: %v", err)
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if _, err := publish(t.Context(), []NativeRelease{release}, "native", bucket); err == nil {
		t.Fatal("missing artifact accepted")
	}
	if len(fake.Written()) != 0 {
		t.Fatal("partially verified release was uploaded")
	}
	partial, _ := nativeFixture(t, commandproto.Targets[:2])
	if _, err := publish(
		t.Context(),
		[]NativeRelease{partial},
		"native",
		bucket,
	); err == nil ||
		!strings.Contains(err.Error(), "it lacks a target: "+commandproto.Targets[2]) {
		t.Fatalf("partial published: %v", err)
	}
	if len(fake.Written()) != 0 {
		t.Fatal("partial release uploaded")
	}
	if err := os.WriteFile(path, original, 0o600); err != nil {
		t.Fatal(err)
	}
	key := "native/blobs/" + descriptor.Targets[commandproto.Targets[5]].SHA256
	if err := bucket.WriteAll(t.Context(), key, []byte("other bytes"), nil); err != nil {
		t.Fatal(err)
	}
	_, err = publish(t.Context(), []NativeRelease{release}, "native", bucket)
	if !errors.Is(err, ErrArtifactConflict) {
		t.Fatalf("squatted artifact: %v", err)
	}
	for _, key := range fake.Written() {
		if !strings.HasPrefix(key, "native/blobs/") {
			t.Fatal("descriptor published over conflict", key)
		}
	}
}

func TestPublishedDownloadLocation(t *testing.T) {
	t.Setenv("AWS_ACCESS_KEY_ID", "fixture")
	t.Setenv("AWS_SECRET_ACCESS_KEY", "fixture")
	t.Setenv("AWS_SESSION_TOKEN", "")
	// Signing uses only fixture credentials, never a vendor request. Virtual time
	// pins the expiry without an execution-speed tolerance.
	synctest.Test(t, func(t *testing.T) {
		bucket, err := (blobs.S3Config{Bucket: "demi-native", Region: "us-east-1"}).Open(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		defer func() {
			if err := bucket.Close(); err != nil {
				t.Error(err)
			}
		}()
		artifact := commandproto.PackageArtifact{SHA256: strings.Repeat("0", 64), Size: 10}
		signed := &SignedArtifacts{signer: bucket, prefix: "native", published: map[string]uint64{artifact.SHA256: 10}}

		asked := time.Now()
		location, err := signed.Resolve(t.Context(), artifact, commandproto.Targets[0])
		if err != nil {
			t.Fatal(err)
		}
		download, ok := location.(*commandproto.ArtifactURL)
		if !ok {
			t.Fatalf("location %T", location)
		}
		parsed, err := url.Parse(download.URL)
		if err != nil {
			t.Fatal(err)
		}
		if parsed.Scheme != "https" || !strings.HasSuffix(parsed.Path, "/native/blobs/"+artifact.SHA256) ||
			parsed.Query().Get("X-Amz-Expires") != "300" {
			t.Fatal(download.URL)
		}
		if download.ExpiresAt == nil || *download.ExpiresAt != asked.Add(300*time.Second).UnixMilli() {
			t.Fatal("expiry differs from five minutes")
		}
		artifact.Size++
		if _, err := signed.Resolve(
			t.Context(),
			artifact,
			"",
		); err == nil ||
			!strings.Contains(err.Error(), "not in the published") {
			t.Fatalf("unpublished accepted: %v", err)
		}
	})
}

func TestNativeConfiguration(t *testing.T) {
	path := filepath.Join(t.TempDir(), "native.json")
	write := func(text string) {
		t.Helper()
		if err := os.WriteFile(path, []byte(text), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	write(
		`{"releases":[{"directory":"./demi-file","executable":"demi-file"}]` +
			`,"store":{"provider":"s3","bucket":"demi-native","region":"us-east-1"}}`,
	)
	config, err := readNativeConfig(t.Context(), path)
	if err != nil {
		t.Fatal(err)
	}
	if config.prefix() != "native" || config.Releases[0].Directory != filepath.Join(filepath.Dir(path), "demi-file") {
		t.Fatalf("config %+v", config)
	}
	// The prefix must begin with a letter or digit.
	write(`{"prefix":"_native/-artifacts","releases":[],"store":{"provider":"s3","bucket":"b","region":"r"}}`)
	if _, err := readNativeConfig(t.Context(), path); err == nil {
		t.Fatal("prefix beginning with underscore accepted")
	}
	write(`{"releases":[{"directory":"demi-file","executable":"demi-file"}],"store":{"provider":"local"}}`)
	config, err = readNativeConfig(t.Context(), path)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := config.Store.(*LocalNativeStore); !ok {
		t.Fatal("not a local store")
	}
	for _, refused := range []string{
		`{"prefix":"native","releases":[{"directory":"d","executable":"a"}],"store":{"provider":"local"}}`,
		`{"releases":[{"directory":"d","executable":"a"}],"store":{"provider":"local","bucket":"b"}}`,
		`{"store":{"provider":"s3","bucket":"b","region":"r"}}`,
		`{"releases":[{"directory":"d","executable":"a.exe"}],"store":{"provider":"s3","bucket":"b","region":"r"}}`,
		`{"releases":[{"directory":"d","executable":"a"}],"store":{"provider":"gcs","bucket":"b","region":"r"}}`,
		`{"prefix":"native/","releases":[{"directory":"d","executable":"a"}]` +
			`,"store":{"provider":"s3","bucket":"b","region":"r"}}`,
		`{"prefix":"a//b","releases":[{"directory":"d","executable":"a"}]` +
			`,"store":{"provider":"s3","bucket":"b","region":"r"}}`,
		`{"releases":[{"directory":"d","executable":"a"}]` +
			`,"store":{"provider":"s3","bucket":"b","region":"r","endpoint":"http://s3.test"}}`,
	} {
		write(refused)
		if _, err := readNativeConfig(t.Context(), path); err == nil {
			t.Fatalf("accepted %s", refused)
		}
	}
}

func TestLocalPublicationDownloadsExecutablesAndResources(t *testing.T) {
	release, descriptor := nativeFixture(t, commandproto.Targets[:1])
	resource := []byte("resource archive bytes")
	digest := fmt.Sprintf("%x", sha256.Sum256(resource))
	descriptor.Resources = map[string]commandproto.PackageResource{
		"bundle": {
			Title: "Bundle",
			Targets: map[string]commandproto.ResourceArtifact{
				commandproto.Targets[0]: {SHA256: digest, Size: uint64(len(resource)), Entry: "file"},
			},
		},
	}
	if err := os.Mkdir(filepath.Join(release.Directory, "resources"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(release.Directory, "resources", digest), resource, 0o600); err != nil {
		t.Fatal(err)
	}
	writeDescriptor(t, release.Directory, descriptor)
	config := NativeConfig{Releases: []NativeRelease{release}, Store: &LocalNativeStore{}}
	data, err := config.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "native.json")
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	catalog, err := PublishNative(t.Context(), path)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := catalog.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	var address PublicURL
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		artifact, ok, err := catalog.LocalArtifact(
			r.Context(),
			strings.TrimPrefix(r.URL.Path, NativeArtifactsRoute+"/"),
		)
		if err != nil {
			t.Error(err)
			http.Error(w, err.Error(), 500)
			return
		}
		if !ok {
			http.NotFound(w, r)
			return
		}
		switch artifact := artifact.(type) {
		case *EncodedArtifact:
			w.Header().Set("Content-Encoding", artifacts.ContentCoding)
			if _, err := w.Write(artifact.Bytes); err != nil {
				t.Error(err)
			}
		case *PlainArtifact:
			http.ServeFile(w, r, artifact.Path)
		}
	}))
	defer server.Close()
	public, err := url.Parse(server.URL + "/base")
	if err != nil {
		t.Fatal(err)
	}
	address.Listening(public, netip.AddrPort{})
	resolver := catalog.Resolver(&address)
	client := artifacts.NewClientAllowingHTTP()
	defer client.Close()
	for _, artifact := range []commandproto.PackageArtifact{
		descriptor.Targets[commandproto.Targets[0]],
		{
			SHA256: digest,
			Size:   uint64(len(resource)),
		},
	} {
		location, err := resolver.Resolve(t.Context(), artifact, commandproto.Targets[0])
		if err != nil {
			t.Fatal(err)
		}
		download, ok := location.(*commandproto.ArtifactURL)
		if !ok {
			t.Fatalf("location %T", location)
		}
		if download.ExpiresAt != nil || strings.Contains(download.URL, "/base/") {
			t.Fatal("wrong local location", download)
		}
		var output bytes.Buffer
		if err := artifacts.Download(
			t.Context(),
			client,
			download.URL,
			artifacts.Digest{SHA256: artifact.SHA256, Size: artifact.Size},
			&output,
		); err != nil {
			t.Fatal(err)
		}
	}
	// Concurrent first requests share one immutable encoding and leave no worker.
	fresh := NewLocalArtifacts(
		[]ArtifactFile{
			{
				Path:     filepath.Join(release.Directory, commandproto.Targets[0], "commands"),
				Artifact: descriptor.Targets[commandproto.Targets[0]],
			},
		},
		nil,
	)
	var workers sync.WaitGroup
	for range 8 {
		workers.Go(func() {
			if _, _, err := fresh.Artifact(
				t.Context(),
				descriptor.Targets[commandproto.Targets[0]].SHA256,
			); err != nil {
				t.Error(err)
			}
		})
	}
	workers.Wait()
	if !catalog.Serves(descriptor.ID, []string{"fixture"}) || catalog.Serves(descriptor.ID, []string{"missing"}) {
		t.Fatal("catalog operation selection changed")
	}
	owned, _ := catalog.Package(descriptor.ID)
	owned.Operations[0] = "mutated"
	delete(owned.Targets, commandproto.Targets[0])
	current, _ := catalog.Package(descriptor.ID)
	if !catalog.Serves(descriptor.ID, []string{"fixture"}) || len(current.Targets) != 1 {
		t.Fatal("caller mutated catalog")
	}
}
