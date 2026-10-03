package runner

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/runnerwire"
)

// A private temporary directory is the only resource these checks use.
func TestInstallationLockReleasedWithActiveRecord(t *testing.T) {
	root := t.TempDir()
	if err := openInstallation(t.Context(), root); err != nil {
		t.Fatal(err)
	}
	first, err := tryInstallationLock(root)
	if err != nil || first == nil {
		t.Fatalf("first lock: %v", err)
	}
	defer func() {
		if err := first.close(); err != nil {
			t.Error(err)
		}
	}()
	second, err := tryInstallationLock(root)
	if err != nil || second != nil {
		t.Fatalf("second lock acquired: %v", err)
	}
	active := filepath.Join(root, "active.json")
	if err := os.WriteFile(active, []byte("record"), 0o600); err != nil {
		t.Fatal(err)
	}
	first.active = active
	if err := first.close(); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(active); !os.IsNotExist(err) {
		t.Fatalf("active record remains: %v", err)
	}
	next, err := tryInstallationLock(root)
	if err != nil || next == nil {
		t.Fatalf("next runner cannot acquire: %v", err)
	}
	if err := next.close(); err != nil {
		t.Fatal(err)
	}
}

func TestInstallationIdentityUsesNormalizedBackend(t *testing.T) {
	first, err := runnerwire.ParseBackendURL("HTTP://LOCALHOST:80")
	if err != nil {
		t.Fatal(err)
	}
	second, err := runnerwire.ParseBackendURL("http://localhost/")
	if err != nil {
		t.Fatal(err)
	}
	if instanceID(first) != instanceID(second) {
		t.Fatal("equivalent backends select different installations")
	}
}
