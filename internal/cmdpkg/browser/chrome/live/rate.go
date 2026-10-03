package live

import "math"

var (
	frameRates = [...]uint32{60, 30, 15}
	scales     = [...]float64{1, .75, .5}
)

const minimumBitrate = 100_000

type sample struct {
	now, ackAge, roundTrip           float64
	bufferedBytes                    int64
	decodeQueue                      uint64
	activeFrames                     uint32
	congested                        bool
	encodedBitrate, deliveredBitrate float64
}

type rate struct {
	minimumRoundTrip, previousRoundTrip, backoffUntil float64
	pixels, budget                                    float64
	frameRate, resolution                             int
	window                                            uint32
}

// roundedBitrate rounds to the nearest integer and saturates at math.MaxUint32.
func roundedBitrate(value float64) uint32 { return uint32(min(math.Round(value), math.MaxUint32)) }

func initialBitrate(pixels uint64, fps uint32) uint32 {
	return roundedBitrate(max(float64(pixels)*float64(fps)*.6, minimumBitrate))
}

func newRate(pixels uint64) rate {
	r := rate{minimumRoundTrip: math.Inf(1), window: 4}
	r.resize(pixels)
	return r
}

func (r *rate) resize(pixels uint64) {
	r.pixels = float64(pixels)
	if r.backoffUntil == 0 {
		r.budget = float64(initialBitrate(pixels, r.fps()))
	}
}
func (r *rate) bitrate() uint32           { return roundedBitrate(r.budget) }
func (r *rate) fps() uint32               { return frameRates[r.frameRate] }
func (r *rate) scale() float64            { return scales[r.resolution] }
func (r *rate) acknowledged(trip float64) { r.minimumRoundTrip = min(r.minimumRoundTrip, trip) }
func (r *rate) bits(fps uint32, scale float64) float64 {
	return r.budget / max(r.pixels*scale*scale*float64(fps), 1)
}

func (r *rate) update(s sample) string {
	delayed := !math.IsInf(r.minimumRoundTrip, 1) && s.roundTrip > r.minimumRoundTrip+150 &&
		s.roundTrip > r.previousRoundTrip+25
	previous := r.previousRoundTrip
	if s.roundTrip > 0 {
		r.previousRoundTrip = s.roundTrip
	}
	cause := ""
	switch {
	case s.congested:
		cause = "frames dropped for the viewer"
	case s.bufferedBytes > 256*1024:
		cause = "bytes waiting for the stream"
	case s.ackAge > max(r.previousRoundTrip+250, 500):
		cause = "an old unacknowledged frame"
	case delayed:
		cause = "a growing round trip"
	case s.decodeQueue > 3:
		cause = "the page's decode queue"
	}
	fell := ""
	fps := float64(r.fps())
	demand := 0.0
	if s.activeFrames > 0 {
		demand = s.encodedBitrate * fps / float64(s.activeFrames)
	}
	if cause != "" {
		fell = r.backoff(s, demand, cause)
	} else if s.now >= r.backoffUntil {
		r.recoverRate(s, demand)
	}
	if cause == "" && s.activeFrames > 0 && math.Abs(s.roundTrip-previous) < 25 {
		r.window = uint32(min(max(math.Ceil(fps*(s.roundTrip+50)/1000), 4), 60))
	}
	return fell
}

func (r *rate) backoff(s sample, demand float64, cause string) string {
	fell := ""
	if !(s.now >= r.backoffUntil) {
		return fell
	}
	delivered := math.Inf(1)
	if s.deliveredBitrate > 0 && demand >= r.budget*.65 {
		delivered = s.deliveredBitrate * .85
	}
	r.budget = math.Round(max(min(r.budget*.7, delivered), minimumBitrate))
	r.backoffUntil = s.now + max(r.previousRoundTrip*2, 1000)
	fell = cause
	if r.bits(r.fps(), r.scale()) < .05 {
		if r.frameRate+1 < len(frameRates) {
			r.frameRate++
		} else if r.resolution+1 < len(scales) {
			r.resolution++
		}
	}
	return fell
}

func (r *rate) recoverRate(s sample, demand float64) {
	if s.activeFrames >= 10 && demand >= r.budget*.65 {
		factor := 1.5
		if r.backoffUntil > 0 && s.now < r.backoffUntil+10_000 {
			factor = 1.1
		}
		r.budget = math.Round(r.budget * factor)
	}
	if r.resolution > 0 && r.bits(r.fps(), scales[r.resolution-1]) >= .15 {
		r.resolution--
	} else if r.resolution == 0 && r.frameRate > 0 && r.bits(frameRates[r.frameRate-1], 1) >= .15 {
		r.frameRate--
	}
}
