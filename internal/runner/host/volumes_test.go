package host

import (
	"context"
	"math"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/runnerwire"
)

func TestGrowthReserveFractionFloorAndSmallCap(t *testing.T) {
	const mib = 1024 * 1024
	for _, test := range []struct{ total, free, want uint64 }{{0, 0, 0}, {128, 32, 0}, {128, 31, 256}, {2048, 255, 4096}, {8192, 820, 0}, {8192, 819, 16384}} {
		result, err := GrowthWanted(test.total*mib, test.free*mib)
		if err != nil {
			t.Fatal(err)
		}
		if test.want == 0 {
			if result != nil {
				t.Fatal(*result)
			}
		} else if result == nil || *result != test.want*mib {
			t.Fatalf("%+v: %v", test, result)
		}
	}
	if _, err := GrowthWanted(math.MaxUint64, 0); err == nil {
		t.Fatal("overflow accepted")
	}
}

func TestVolumeGrowthHasOneRequestUntilMatchingReply(t *testing.T) {
	out := make(chan []byte, 4)
	v := NewVolumes(t.Context(), []ManagedVolume{{Name: runnerwire.VolumeNameHome, Mount: t.TempDir()}}, out)
	t.Cleanup(func() {
		if err := v.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	// Supply one completed capacity observation. The state machine is tested
	// without filling the developer's real filesystem to its growth threshold.
	wanted := uint64(512 * 1024 * 1024)
	v.pending[runnerwire.VolumeNameHome] = ""
	v.checks <- volumeCheck{name: runnerwire.VolumeNameHome, wanted: &wanted}
	var jobs sync.WaitGroup
	results := make(chan bool, 2)
	for range 2 {
		jobs.Go(func() {
			checked, err := v.Checked(t.Context())
			if err != nil {
				t.Error(err)
			}
			results <- checked
		})
	}
	jobs.Wait()
	first, second := <-results, <-results
	if first == second {
		t.Fatalf("one observation consumed twice or not at all: %v %v", first, second)
	}
	frame, ok := receiveFrame(t, out).(*runnerwire.VolumeGrow)
	if !ok || frame.Bytes != wanted || frame.Volume != runnerwire.VolumeNameHome || len(frame.ID) != 32 {
		t.Fatal(frame)
	}
	v.Poll()
	if checked, err := v.Checked(t.Context()); err != nil || checked {
		t.Fatalf("duplicate growth: %v %v", checked, err)
	}
	if err := v.Grown(runnerwire.VolumeGrown{ID: "wrong", Volume: runnerwire.VolumeNameHome, Bytes: wanted}); err == nil {
		t.Fatal("wrong reply accepted")
	}
	if err := v.Grown(runnerwire.VolumeGrown{ID: frame.ID, Volume: frame.Volume, Bytes: frame.Bytes}); err != nil {
		t.Fatal(err)
	}
	if err := v.Grown(runnerwire.VolumeGrown{ID: frame.ID, Volume: frame.Volume, Bytes: frame.Bytes}); err == nil {
		t.Fatal("duplicate reply accepted")
	}
}
