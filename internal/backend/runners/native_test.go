package runners_test

import (
	"context"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/backend/runners"
)

func TestEmptyNativeCatalog(t *testing.T) {
	// Opening the production S3 client must not cause an upload for an empty list.
	t.Setenv("AWS_ACCESS_KEY_ID", "fixture")
	t.Setenv("AWS_SECRET_ACCESS_KEY", "fixture")
	path := filepath.Join(t.TempDir(), "native.json")
	if err := os.WriteFile(
		path,
		[]byte(`{"releases":[],"store":{"provider":"s3","bucket":"demi-native","region":"us-east-1"}}`),
		0o600,
	); err != nil {
		t.Fatal(err)
	}
	catalog, err := runners.PublishNative(t.Context(), path)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := catalog.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	if _, ok := catalog.Package("demi.browser"); ok || catalog.Serves("demi.browser", nil) {
		t.Fatal("empty release serves a package")
	}
}
