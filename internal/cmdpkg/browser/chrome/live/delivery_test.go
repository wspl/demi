package live

import (
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// Delivery tests exercise the bytes and capture pacing a viewer observes, in
// virtual time; they use no sockets and cost less than one second each.
func deliveryFixture() (*writer, *streamView) {
	w := &writer{controls: make(chan []byte, 64), videos: make(chan []byte, 4), done: make(chan struct{})}
	stream := &captureStream{wake: make(chan struct{}, 1)}
	return w, &streamView{stream: stream}
}
func TestDeliveryDropsBrokenPicturesAndResumesAtKeyFrame(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		w, stream := deliveryFixture()
		d := newDelivery()
		d.picture(t.Context(), "t1", stream, picture{kind: pictureRestart, epoch: 3, width: 1280, height: 720}, w)
		d.picture(t.Context(), "t1", stream, picture{kind: pictureFrame, frame: &tabs.Frame{Sequence: 1, Width: 1280, Height: 720, Data: []byte{1}}}, w)
		if len(w.videos) != 0 || stream.pace.floor != 1 {
			t.Fatal("delta before key frame held the capture or reached viewer")
		}
		frame := tabs.Frame{Sequence: 2, Key: true, Width: 1280, Height: 720, Data: []byte{2}}
		d.picture(t.Context(), "t1", stream, picture{kind: pictureFrame, frame: &frame}, w)
		data := <-w.videos
		header, payload, err := browserop.SplitVideoFrame(data[5:])
		if err != nil || header.Generation != 1 || header.Sequence != 2 || !header.Key || string(payload) != string([]byte{2}) {
			t.Fatalf("video = %+v, %v, %v", header, payload, err)
		}
		if stream.pace.floor != 1 || stream.pace.epoch != 3 {
			t.Fatalf("pace = %+v", stream.pace)
		}
		time.Sleep(20 * time.Millisecond)
		d.ack(0, 2, 0, stream)
		if stream.pace.floor != 1 {
			t.Fatal("stale generation acknowledged a current frame")
		}
		d.ack(1, 2, 0, stream)
		if stream.pace.floor != 2 || d.roundTrip != 20 {
			t.Fatalf("ack pace = %+v, round trip = %v", stream.pace, d.roundTrip)
		}
		frame.Sequence = 4
		frame.Key = false
		d.picture(t.Context(), "t1", stream, picture{kind: pictureFrame, frame: &frame}, w)
		if len(w.videos) != 0 || stream.pace.keys != 1 {
			t.Fatal("missing delta did not request a key frame")
		}
		frame.Sequence = 5
		frame.Key = true
		d.picture(t.Context(), "t1", stream, picture{kind: pictureFrame, frame: &frame}, w)
		if len(w.videos) != 1 {
			t.Fatal("key frame did not resume delivery")
		}
	})
}
func TestDeliveryBackpressureRecoversWithoutReplayingFrames(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		w, stream := deliveryFixture()
		d := newDelivery()
		d.picture(t.Context(), "t1", stream, picture{kind: pictureRestart, epoch: 1, width: 100, height: 100}, w)
		for sequence := uint32(1); sequence <= 5; sequence++ {
			d.picture(t.Context(), "t1", stream, picture{kind: pictureFrame, frame: &tabs.Frame{Sequence: sequence, Key: sequence == 1, Width: 100, Height: 100}}, w)
		}
		if len(w.videos) != 4 || !d.dropped || stream.pace.keys != 1 {
			t.Fatal("full video queue did not drop and request recovery")
		}
		for len(w.videos) > 0 {
			<-w.videos
		}
		d.picture(t.Context(), "t1", stream, picture{kind: pictureFrame, frame: &tabs.Frame{Sequence: 6, Width: 100, Height: 100}}, w)
		if len(w.videos) != 0 {
			t.Fatal("delta after overflow reached viewer")
		}
		d.picture(t.Context(), "t1", stream, picture{kind: pictureFrame, frame: &tabs.Frame{Sequence: 7, Key: true, Width: 100, Height: 100}}, w)
		header, _, err := browserop.SplitVideoFrame((<-w.videos)[5:])
		if err != nil || header.Sequence != 7 {
			t.Fatalf("recovery frame = %+v, %v", header, err)
		}
	})
}
func TestDeliveryStallReleasesCaptureWithoutLoweringBudget(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		w, stream := deliveryFixture()
		d := newDelivery()
		budget := d.rate.bitrate()
		time.Sleep(time.Millisecond)
		d.picture(t.Context(), "t1", stream, picture{kind: pictureFrame, frame: &tabs.Frame{Sequence: 1, Key: true, Width: 1280, Height: 720}}, w)
		// A Host pause longer than three ticks is not path congestion.
		time.Sleep(4 * time.Second)
		d.tick(stream, w)
		if d.rate.bitrate() != budget || stream.pace.floor != 1 || !d.awaitingKey || len(d.flight) != 0 {
			t.Fatalf("stall budget=%d pace=%+v awaiting=%v", d.rate.bitrate(), stream.pace, d.awaitingKey)
		}
	})
}

func TestDeliveryAdaptsImmediatelyAndAfterOneSecond(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		w, stream := deliveryFixture()
		d := newDelivery()
		started := time.Now()
		ticks := d.startTicks(stream, w)
		defer ticks.Stop()
		if stream.pace.encoding == nil || stream.pace.encoding.bitrate != 33_177_600 || time.Since(started) != 0 {
			t.Fatalf("initial adaptation = %+v at %v", stream.pace.encoding, time.Since(started))
		}

		// Sustained encoder demand must raise quality on the very next tick.
		d.frames = 60
		d.bytes = 4_147_200
		time.Sleep(time.Second - time.Nanosecond)
		select {
		case <-ticks.C:
			t.Fatal("periodic adaptation arrived before one second")
		default:
		}
		<-ticks.C
		d.tick(stream, w)
		if stream.pace.encoding.bitrate != 49_766_400 || time.Since(started) != time.Second {
			t.Fatalf("next adaptation = %+v at %v", stream.pace.encoding, time.Since(started))
		}
	})
}
