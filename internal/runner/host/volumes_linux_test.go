package host

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/runnerwire"
)

func TestLinuxVolumeUsageFlushAndChecks(t *testing.T) {
	root := t.TempDir()
	total, available, err := volumeUsage(root)
	if err != nil || total == 0 || available > total {
		t.Fatalf("%d %d %v", total, available, err)
	}
	out := make(chan []byte, 4)
	v := NewVolumes(t.Context(), []ManagedVolume{{Name: runnerwire.VolumeNameHome, Mount: root}}, out)
	t.Cleanup(func() {
		if err := v.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	if err := v.Sync(t.Context(), runnerwire.Sync{ID: "flush"}); err != nil {
		t.Fatal(err)
	}
	frame, ok := receiveFrame(t, out).(*runnerwire.SyncDone)
	if !ok || frame.Error != nil || frame.ID != "flush" {
		t.Fatal(frame)
	}
	v.Poll()
	if checked, err := v.Checked(t.Context()); err != nil || !checked {
		t.Fatalf("%v %v", checked, err)
	}
	if checked, err := v.Checked(t.Context()); err != nil || checked {
		t.Fatalf("%v %v", checked, err)
	}
	if err := v.Grown(runnerwire.VolumeGrown{ID: "wrong", Volume: runnerwire.VolumeNameHome}); err == nil {
		t.Fatal("unknown request accepted")
	}
}
