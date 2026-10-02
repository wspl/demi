package live

import (
	"context"
	"errors"
	"log/slog"
	"sync"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

type pictureKind uint8

const (
	pictureRestart pictureKind = iota
	pictureFrame
	pictureUnavailable
	pictureFailed
)

type picture struct {
	kind                 pictureKind
	epoch, width, height uint32
	frame                *tabs.Frame
	reason               string
}

type encoding struct {
	bitrate, fps uint32
	scale        float64
}
type pacing struct {
	epoch, floor, window uint32
	paced                bool
	encoding             *encoding
	keys                 uint64
}
type streamView struct {
	stream *captureStream
	events chan picture
	pace   pacing
}

func (v *streamView) change(change func(*pacing)) {
	v.stream.mu.Lock()
	change(&v.pace)
	v.stream.mu.Unlock()
	v.stream.wakeUp()
}
func (v *streamView) paced(epoch, floor, window uint32) {
	v.change(func(p *pacing) {
		p.epoch = epoch
		p.floor = floor
		p.window = window
		p.paced = true
	})
}
func (v *streamView) encode(e encoding) { v.change(func(p *pacing) { p.encoding = &e }) }
func (v *streamView) keyFrame()         { v.change(func(p *pacing) { p.keys++ }) }

type captureStream struct {
	// mu protects membership and the latest pacing; no IO runs under it.
	mu      sync.Mutex
	members map[uint64]*streamView
	wake    chan struct{}
	done    chan struct{}
	cancel  context.CancelFunc
}

func startStream(ctx context.Context, tab *tabs.Tab, captures *tabs.CaptureChannel) (*captureStream, error) {
	ctx, cancel := context.WithCancel(ctx)
	s := &captureStream{members: make(map[uint64]*streamView), wake: make(chan struct{}, 1), done: make(chan struct{}), cancel: cancel}
	err := tab.StartTask(func(tabCtx context.Context) {
		defer close(s.done)
		defer cancel()
		stop := context.AfterFunc(tabCtx, cancel)
		defer stop()
		s.run(ctx, tab, captures)
	})
	if err != nil {
		cancel()
		return nil, err
	}
	return s, nil
}
func (s *captureStream) wakeUp() {
	select {
	case s.wake <- struct{}{}:
	default:
	}
}
func (s *captureStream) join(id uint64) *streamView {
	v := &streamView{stream: s, events: make(chan picture, 8)}
	s.mu.Lock()
	s.members[id] = v
	s.mu.Unlock()
	s.wakeUp()
	return v
}
func (s *captureStream) leave(id uint64) bool {
	s.mu.Lock()
	delete(s.members, id)
	empty := len(s.members) == 0
	s.mu.Unlock()
	s.wakeUp()
	return empty
}

// close cancels capture and joins its worker even if the owner is canceled.
func (s *captureStream) close(_ context.Context) {
	s.cancel()
	<-s.done
}

type streamMember struct {
	view          *streamView
	floor, window uint32
	encoding      encoding
	keys          uint64
}

func (v *streamMember) send(p picture) {
	select {
	case v.view.events <- p:
	default:
	}
}

type captureResult struct {
	event tabs.CaptureEvent
	err   error
}
type runningCapture struct {
	capture                        *tabs.Capture
	cancel                         context.CancelFunc
	done                           chan struct{}
	events                         chan captureResult
	epoch, width, height, sequence uint32
	encoding                       encoding
}

func (r *runningCapture) close(ctx context.Context) {
	r.cancel()
	<-r.done
	if err := r.capture.Close(context.WithoutCancel(ctx)); err != nil {
		slog.Warn("live capture close", "error", err)
	}
}
func beginCapture(ctx context.Context, capture *tabs.Capture, epoch, width, height uint32, e encoding) *runningCapture {
	ctx, cancel := context.WithCancel(ctx)
	r := &runningCapture{capture: capture, cancel: cancel, done: make(chan struct{}), events: make(chan captureResult), epoch: epoch, width: width, height: height, encoding: e}
	go func() {
		defer close(r.done)
		for {
			event, err := capture.Next(ctx)
			select {
			case r.events <- captureResult{event, err}:
			case <-ctx.Done():
				return
			}
			if err != nil {
				return
			}
		}
	}()
	return r
}
func (s *captureStream) run(ctx context.Context, tab *tabs.Tab, captures *tabs.CaptureChannel) {
	viewers := make(map[uint64]*streamMember)
	var running *runningCapture
	defer func() {
		if running != nil {
			running.close(ctx)
		}
	}()
	epoch := uint32(0)
	retry := time.Duration(0)
	attempt := time.Now()
	var unavailable, failure *string
	timer := time.NewTimer(time.Hour)
	defer timer.Stop()
	report := func(reason string) {
		if failure != nil && *failure == reason {
			return
		}
		failure = &reason
		for _, v := range viewers {
			v.send(picture{kind: pictureFailed, reason: reason})
		}
	}
	for {
		if ctx.Err() != nil {
			return
		}
		viewport, changed := tab.Viewports()
		desired := encoding{fps: 60, scale: 1}
		for _, v := range viewers {
			desired.fps = min(desired.fps, v.encoding.fps)
			desired.scale = min(desired.scale, v.encoding.scale)
			if v.encoding.bitrate != 0 && (desired.bitrate == 0 || v.encoding.bitrate < desired.bitrate) {
				desired.bitrate = v.encoding.bitrate
			}
		}
		width, height := tabs.Pixels(viewport.Current, desired.scale)
		if desired.bitrate == 0 {
			desired.bitrate = initialBitrate(uint64(width)*uint64(height), desired.fps)
		}
		if len(viewers) == 0 && running != nil {
			running.close(ctx)
			running = nil
		}
		restart := running == nil || running.width != width || running.height != height
		if len(viewers) > 0 && restart && unavailable == nil && !time.Now().Before(attempt) {
			if running != nil {
				running.close(ctx)
				running = nil
			}
			capture, err := captures.Start(ctx, tab.TargetID(), width, height, desired.fps, desired.bitrate)
			if err == nil {
				epoch++
				running = beginCapture(ctx, capture, epoch, width, height, desired)
				for _, v := range viewers {
					v.floor = 0
					v.send(picture{kind: pictureRestart, epoch: epoch, width: width, height: height})
				}
			} else {
				var browserErr *cdp.BrowserError
				if errors.As(err, &browserErr) && browserErr.Kind == cdp.KindUnsupportedCapability {
					reason := browserErr.Message
					unavailable = &reason
					for _, v := range viewers {
						v.send(picture{kind: pictureUnavailable, reason: *unavailable})
					}
				} else {
					slog.Warn("live view capture", "tab", tab.ID(), "error", err)
					report(err.Error())
					retry = min(max(retry*2, 500*time.Millisecond), 5*time.Second)
					attempt = time.Now().Add(retry)
				}
			}
		}
		if running != nil && (running.encoding.bitrate != desired.bitrate || running.encoding.fps != desired.fps) {
			running.capture.Encoding(desired.bitrate, desired.fps)
			running.encoding = desired
		}
		var retryAt <-chan time.Time
		timer.Stop()
		if running == nil && unavailable == nil && len(viewers) > 0 {
			timer.Reset(time.Until(attempt))
			retryAt = timer.C
		}
		var events <-chan captureResult
		if running != nil {
			events = running.events
		}
		select {
		case <-ctx.Done():
			return
		case <-changed:
		case <-retryAt:
		case <-s.wake:
			s.mu.Lock()
			current := make(map[uint64]struct {
				view *streamView
				pace pacing
			}, len(s.members))
			for id, v := range s.members {
				current[id] = struct {
					view *streamView
					pace pacing
				}{v, v.pace}
			}
			s.mu.Unlock()
			for id := range viewers {
				if _, ok := current[id]; !ok {
					delete(viewers, id)
				}
			}
			key, ack := false, false
			for id, item := range current {
				v := viewers[id]
				if v == nil {
					v = &streamMember{view: item.view, window: 4, encoding: encoding{fps: 60, scale: 1}}
					viewers[id] = v
					if unavailable != nil {
						v.send(picture{kind: pictureUnavailable, reason: *unavailable})
					}
					if failure != nil {
						v.send(picture{kind: pictureFailed, reason: *failure})
					}
					if running != nil {
						v.floor = running.sequence
						v.send(picture{kind: pictureRestart, epoch: running.epoch, width: running.width, height: running.height})
						key = true
					}
				}
				p := item.pace
				if p.paced && running != nil && p.epoch == running.epoch {
					v.floor, v.window = p.floor, p.window
					ack = true
				}
				if p.encoding != nil {
					v.encoding = *p.encoding
				}
				if p.keys != v.keys {
					v.keys = p.keys
					key = true
				}
			}
			if running != nil {
				if ack && len(viewers) > 0 {
					floor, window := ^uint32(0), ^uint32(0)
					for _, v := range viewers {
						floor = min(floor, v.floor)
						window = min(window, v.window)
					}
					running.capture.Ack(floor, window)
				}
				if key {
					running.capture.KeyFrame()
				}
			}
		case result := <-events:
			if result.err != nil {
				running.close(ctx)
				running = nil
				retry = min(max(retry*2, 500*time.Millisecond), 5*time.Second)
				attempt = time.Now().Add(retry)
				continue
			}
			switch event := result.event.(type) {
			case *tabs.CaptureFrame:
				retry = 0
				failure = nil
				running.sequence = event.Frame.Sequence
				for _, v := range viewers {
					v.send(picture{kind: pictureFrame, frame: &event.Frame})
				}
			case *tabs.CaptureStarted:
			case *tabs.CaptureStalled:
				if err := tab.StartTask(func(paintCtx context.Context) {
					paintCtx, cancel := context.WithTimeout(paintCtx, 5*time.Second)
					defer cancel()
					// A failed repaint is recoverable through capture's silence handling.
					if err := tabs.Paint(paintCtx, tab.Page()); err != nil {
						slog.Debug("live capture repaint", "error", err)
					}
				}); err != nil {
					slog.Debug("live capture repaint admission", "error", err)
				}
			case *tabs.CaptureFailed:
				slog.Warn("live view capture", "tab", tab.ID(), "error", event.Reason)
				report(event.Reason)
				running.close(ctx)
				running = nil
				retry = min(max(retry*2, 500*time.Millisecond), 5*time.Second)
				attempt = time.Now().Add(retry)
			}
		}
	}
}
