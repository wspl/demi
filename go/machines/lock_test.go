//go:build linux

package machines_test

import (
	"errors"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/go/machines"
)

func TestASecondManagerIsRefusedWithThePath(t *testing.T) {
	data, runtime := t.TempDir(), t.TempDir()
	first, err := machines.AcquireLocks(data, runtime)
	if err != nil {
		t.Fatal(err)
	}
	_, err = machines.AcquireLocks(data, runtime)
	var owned *machines.OwnedError
	if !errors.As(err, &owned) || err.Error() != "Another Cloud manager owns "+filepath.Join(data, "manager.lock") {
		t.Fatalf("the second manager: %v", err)
	}
	// The runtime directory alone is enough to be refused.
	if _, err := machines.AcquireLocks(t.TempDir(), runtime); !errors.As(err, &owned) || err.Error() != "Another Cloud manager owns "+filepath.Join(runtime, "manager.lock") {
		t.Fatalf("a manager on another state directory: %v", err)
	}
	if err := first.Close(); err != nil {
		t.Fatal(err)
	}
	again, err := machines.AcquireLocks(data, runtime)
	if err != nil {
		t.Fatalf("after the first released: %v", err)
	}
	again.Close()
}
