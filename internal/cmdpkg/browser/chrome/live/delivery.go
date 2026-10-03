package live

import (
	"context"
	"log/slog"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

type flight struct {
	sequence uint32
	sent     time.Time
	bytes    int
}
type delivery struct {
	generation, epoch, last  uint32
	awaitingKey              bool
	flight                   []flight
	rate                     rate
	applied                  *encoding
	started, ticked, lastAck time.Time
	roundTrip                float64
	decodeQueue              uint64
	frames                   uint32
	bytes, acknowledged      int
	dropped                  bool
}

func newDelivery() delivery {
	now := time.Now()
	return delivery{awaitingKey: true, rate: newRate(1280 * 720), started: now, ticked: now, lastAck: now}
}

func (d *delivery) floor() uint32 {
	if len(d.flight) == 0 {
		return d.last
	}
	if d.flight[0].sequence == 0 {
		return 0
	}
	return d.flight[0].sequence - 1
}

func (d *delivery) pace(stream *streamView) {
	if stream != nil {
		stream.paced(d.epoch, d.floor(), d.rate.window)
	}
}

func (d *delivery) picture(ctx context.Context, tab browserop.TabID, stream *streamView, p picture, w *writer) {
	switch p.kind {
	case pictureRestart:
		d.generation++
		d.epoch = p.epoch
		d.awaitingKey = true
		d.last = 0
		d.flight = nil
		d.rate.resize(uint64(p.width) * uint64(p.height))
		w.control(
			ctx,
			&browserop.LiveModuleMessageStream{Tab: tab, Generation: d.generation, Width: p.width, Height: p.height},
		)
	case pictureFrame:
		frame := p.frame
		if frame.Sequence <= d.last {
			return
		}
		if d.last != 0 && frame.Sequence != d.last+1 && !frame.Key {
			d.awaitingKey = true
			if stream != nil {
				stream.keyFrame()
			}
		}
		d.last = frame.Sequence
		if d.awaitingKey && !frame.Key {
			d.pace(stream)
			return
		}
		data, err := videoFrame(tab, d.generation, frame.Sequence, *frame)
		if err != nil {
			slog.Warn("live video frame", "error", err)
			return
		}
		if w.video(data) {
			d.awaitingKey = false
			d.flight = append(d.flight, flight{frame.Sequence, time.Now(), len(data)})
			d.frames++
			d.bytes += len(data)
		} else {
			d.dropped = true
			d.awaitingKey = true
			if stream != nil {
				stream.keyFrame()
			}
		}
		d.pace(stream)
	case pictureUnavailable:
		w.notice(ctx, browserop.CaptureUnavailable, p.reason)
	case pictureFailed:
		w.notice(ctx, browserop.CaptureFailed, p.reason)
	}
}

func (d *delivery) ack(generation, sequence, decodeQueue uint32, stream *streamView) {
	if d.generation != generation {
		return
	}
	now := time.Now()
	for len(d.flight) > 0 && d.flight[0].sequence <= sequence {
		f := d.flight[0]
		d.flight = d.flight[1:]
		d.acknowledged += f.bytes
		if f.sequence == sequence {
			d.roundTrip = now.Sub(f.sent).Seconds() * 1000
			d.rate.acknowledged(d.roundTrip)
		}
	}
	d.decodeQueue = uint64(decodeQueue)
	d.lastAck = now
	d.pace(stream)
}

// startTicks applies the viewer's initial rate immediately, as Tokio's interval
// does, then schedules adaptation every second. The caller stops the ticker.
func (d *delivery) startTicks(stream *streamView, w *writer) *time.Ticker {
	d.tick(stream, w)
	return time.NewTicker(time.Second)
}

func (d *delivery) tick(stream *streamView, w *writer) {
	now := time.Now()
	elapsed := now.Sub(d.ticked)
	d.ticked = now
	stall := time.Duration(browserop.StallMS) * time.Millisecond
	var oldest time.Time
	if len(d.flight) > 0 {
		oldest = d.flight[0].sent
	}
	stalled := elapsed > 3*time.Second || (!oldest.IsZero() && now.Sub(oldest) > stall && d.lastAck.Before(oldest))
	if stalled {
		if !oldest.IsZero() && now.Sub(oldest) > 2*stall {
			d.flight = nil
			d.awaitingKey = true
			d.pace(stream)
		}
	} else {
		d.adaptRate(stream, w, now, oldest, elapsed)
	}
	d.frames = 0
	d.bytes = 0
	d.acknowledged = 0
	d.dropped = false
}

func (d *delivery) adaptRate(stream *streamView, w *writer, now, oldest time.Time, elapsed time.Duration) {
	seconds := max(elapsed.Seconds(), .001)
	s := sample{
		now:              now.Sub(d.started).Seconds() * 1000,
		bufferedBytes:    w.queued.Load(),
		decodeQueue:      d.decodeQueue,
		activeFrames:     d.frames,
		congested:        d.dropped,
		encodedBitrate:   float64(d.bytes) * 8 / seconds,
		deliveredBitrate: float64(d.acknowledged) * 8 / seconds,
	}
	if !oldest.IsZero() {
		s.ackAge = now.Sub(oldest).Seconds() * 1000
	}
	if d.frames > 0 {
		s.roundTrip = d.roundTrip
	}
	if cause := d.rate.update(s); cause != "" {
		slog.Info(
			"live view rate fell",
			"bitrate",
			d.rate.bitrate(),
			"fps",
			d.rate.fps(),
			"scale",
			d.rate.scale(),
			"cause",
			cause,
			"buffered_bytes",
			s.bufferedBytes,
			"ack_age_ms",
			s.ackAge,
			"round_trip_ms",
			s.roundTrip,
			"decode_queue",
			s.decodeQueue,
			"encoded_bitrate",
			s.encodedBitrate,
			"delivered_bitrate",
			s.deliveredBitrate,
		)
	}
	e := encoding{d.rate.bitrate(), d.rate.fps(), d.rate.scale()}
	if d.applied == nil || *d.applied != e {
		d.applied = &e
		if stream != nil {
			stream.encode(e)
		}
	}
}
