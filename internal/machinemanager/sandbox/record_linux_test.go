//go:build linux

package sandbox_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/sandbox"
)

func TestRecordNamesBootAndSlot(t *testing.T) {
	record, err := sandbox.DecodeRecord([]byte(`{"id":"demi-0f6c3d4e-8a9b-4c1d-9e2f-3a4b5c6d7e8f","slot":3}`))
	if err != nil || record.Slot != 3 {
		t.Fatalf("record = %+v, %v", record, err)
	}
	for _, invalid := range []string{
		`{"id":"0f6c3d4e","slot":3}`,
		`{"id":"demi-../x","slot":3}`,
		`{"id":"demi-a","slot":-1}`,
		`{"id":"demi-a","slot":3,"token":"x"}`,
	} {
		if _, err := sandbox.DecodeRecord([]byte(invalid)); err == nil {
			t.Errorf("accepted %s", invalid)
		}
	}
	id, err := sandbox.NewID()
	if err != nil || !strings.HasPrefix(string(id), "demi-") {
		t.Fatalf("new ID = %q, %v", id, err)
	}
}
