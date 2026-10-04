package live

import (
	"reflect"
	"testing"
	"testing/synctest"
	"time"
)

// Rate scenarios use virtual time and no external resources; each costs <1 s.
func healthySample() sample {
	return sample{roundTrip: 5, activeFrames: 30, encodedBitrate: 6_000_000}
}

func referenceRate() rate {
	r := newRate(500 * 1000)
	r.budget = 6_000_000
	return r
}

func wantBitrate(t *testing.T, r *rate, want uint32) {
	t.Helper()
	if got := r.bitrate(); got != want {
		t.Fatalf("bitrate = %d, want %d", got, want)
	}
}

func TestShortScrollingBurstsRaiseQualityWithoutWarmingUp(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		start := time.Now()
		r := referenceRate()
		r.acknowledged(5)
		r.update(healthySample())
		wantBitrate(t, &r, 9_000_000)
		time.Sleep(time.Second)
		s := healthySample()
		s.now = time.Since(start).Seconds() * 1000
		s.activeFrames = 0
		s.encodedBitrate = 0
		r.update(s)
		wantBitrate(t, &r, 9_000_000)
		time.Sleep(time.Second)
		s = healthySample()
		s.now = time.Since(start).Seconds() * 1000
		s.encodedBitrate = 9_000_000
		r.update(s)
		wantBitrate(t, &r, 13_500_000)
	})
}

func TestRecoveryWaitsForFeedbackAndProbesConservatively(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		start := time.Now()
		r := referenceRate()
		s := healthySample()
		s.ackAge = 900
		r.update(s)
		wantBitrate(t, &r, 4_200_000)
		time.Sleep(500 * time.Millisecond)
		s = healthySample()
		s.now = time.Since(start).Seconds() * 1000
		r.update(s)
		wantBitrate(t, &r, 4_200_000)
		time.Sleep(500 * time.Millisecond)
		s.now = time.Since(start).Seconds() * 1000
		r.update(s)
		wantBitrate(t, &r, 4_620_000)
	})
}

func TestFirstBudgetScalesWithDevicePixelsWithoutCeiling(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := newRate(1532 * 862)
		wantBitrate(t, &r, 47_541_024)
		r.resize(2000 * 1400)
		wantBitrate(t, &r, 100_800_000)
		time.Sleep(time.Second)
		s := healthySample()
		s.now = 1000
		s.congested = true
		r.update(s)
		constrained := r.bitrate()
		r.resize(2800 * 1800)
		wantBitrate(t, &r, constrained)
	})
}

func TestIdlePictureIsNoEvidenceOfBandwidth(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := referenceRate()
		s := healthySample()
		s.congested = true
		r.update(s)
		start := time.Now()
		for range 30 {
			time.Sleep(time.Second)
			s = healthySample()
			s.activeFrames = 0
			s.now = time.Since(start).Seconds() * 1000
			r.update(s)
		}
		wantBitrate(t, &r, 4_200_000)
	})
}

func TestLongRoundTripKeepsQualityButStalledAckDoesNot(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := referenceRate()
		r.acknowledged(600)
		s := healthySample()
		s.roundTrip = 600
		s.ackAge = 600
		s.encodedBitrate = 100_000
		r.update(s)
		wantBitrate(t, &r, 6_000_000)
		s = healthySample()
		s.ackAge = 900
		r.update(s)
		wantBitrate(t, &r, 4_200_000)
		start := time.Now()
		for range 30 {
			time.Sleep(2 * time.Second)
			s = healthySample()
			s.now = time.Since(start).Seconds() * 1000
			s.decodeQueue = 5
			r.update(s)
		}
		wantBitrate(t, &r, 100_000)
	})
}

func TestLowEncoderDemandDoesNotRaiseBudget(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := referenceRate()
		start := time.Now()
		for range 300 {
			time.Sleep(time.Second)
			s := healthySample()
			s.encodedBitrate = 100_000
			s.now = time.Since(start).Seconds() * 1000
			r.update(s)
		}
		wantBitrate(t, &r, 6_000_000)
	})
}

func TestIntermittentFramesDoNotHideScrollingDemand(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := referenceRate()
		s := healthySample()
		s.activeFrames = 12
		s.encodedBitrate = 1_200_000
		r.update(s)
		wantBitrate(t, &r, 9_000_000)
	})
}

func TestCongestionLowersBitsFramesPixelsAndRecoveryReverses(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		type quality struct {
			fps   uint32
			scale float64
		}
		r := newRate(2880 * 1800)
		start := time.Now()
		seen := []quality{{r.fps(), r.scale()}}
		for r.bitrate() > 100_000 {
			time.Sleep(2 * time.Second)
			s := healthySample()
			s.now = time.Since(start).Seconds() * 1000
			s.congested = true
			r.update(s)
			q := quality{r.fps(), r.scale()}
			if seen[len(seen)-1] != q {
				seen = append(seen, q)
			}
		}
		want := []quality{{60, 1}, {30, 1}, {15, 1}, {15, .75}, {15, .5}}
		if !reflect.DeepEqual(seen, want) {
			t.Fatalf("congestion path = %v, want %v", seen, want)
		}
		seen = nil
		for r.fps() != 60 || r.scale() != 1 {
			time.Sleep(11 * time.Second)
			s := healthySample()
			s.now = time.Since(start).Seconds() * 1000
			s.encodedBitrate = float64(r.bitrate())
			s.activeFrames = r.fps()
			r.update(s)
			q := quality{r.fps(), r.scale()}
			if len(seen) == 0 || seen[len(seen)-1] != q {
				seen = append(seen, q)
			}
			if time.Since(start) > 1000*time.Second {
				t.Fatal("never recovered")
			}
		}
		want = []quality{{15, .5}, {15, .75}, {15, 1}, {30, 1}, {60, 1}}
		if !reflect.DeepEqual(seen, want) {
			t.Fatalf("recovery path = %v, want %v", seen, want)
		}
	})
}
