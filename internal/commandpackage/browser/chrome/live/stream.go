package live

import (
	"context"
	"errors"
	"log/slog"
	"sync"
	"time"

	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
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
	s := &captureStream{
		members: make(map[uint64]*streamView),
		wake:    make(chan struct{}, 1),
		done:    make(chan struct{}),
		cancel:  cancel,
	}
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

type runningCapture struct {
	capture                        *tabs.Capture
	epoch, width, height, sequence uint32
	encoding                       encoding
}

func (r *runningCapture) close(ctx context.Context) {
	if err := r.capture.Close(context.WithoutCancel(ctx)); err != nil {
		slog.Warn("live capture close", "error", err)
	}
}

func beginCapture(capture *tabs.Capture, epoch, width, height uint32, e encoding) *runningCapture {
	return &runningCapture{capture: capture, epoch: epoch, width: width, height: height, encoding: e}
}

func (s *captureStream) run(ctx context.Context, tab *tabs.Tab, captures *tabs.CaptureChannel) {
	cycle := captureCycle{viewers: make(map[uint64]*streamMember), attempt: time.Now()}
	defer func() {
		if cycle.running != nil {
			cycle.running.close(ctx)
		}
	}()
	timer := time.NewTimer(time.Hour)
	defer timer.Stop()
	for {
		if ctx.Err() != nil {
			return
		}
		viewport, changed := tab.Viewports()
		desired := desiredEncoding(cycle.viewers)
		width, height := tabs.Pixels(viewport.Current, desired.scale)
		if desired.bitrate == 0 {
			desired.bitrate = initialBitrate(uint64(width)*uint64(height), desired.fps)
		}
		if len(cycle.viewers) == 0 && cycle.running != nil {
			cycle.running.close(ctx)
			cycle.running = nil
		}
		restart := cycle.running == nil || cycle.running.width != width || cycle.running.height != height
		if len(cycle.viewers) > 0 && restart && cycle.unavailable == nil && !time.Now().Before(cycle.attempt) {
			cycle.restart(ctx, tab, captures, width, height, desired)
		}
		if cycle.running != nil &&
			(cycle.running.encoding.bitrate != desired.bitrate || cycle.running.encoding.fps != desired.fps) {
			cycle.running.capture.Encoding(desired.bitrate, desired.fps)
			cycle.running.encoding = desired
		}
		var retryAt <-chan time.Time
		timer.Stop()
		if cycle.running == nil && cycle.unavailable == nil && len(cycle.viewers) > 0 {
			timer.Reset(time.Until(cycle.attempt))
			retryAt = timer.C
		}
		var events <-chan tabs.CaptureEvent
		if cycle.running != nil {
			events = cycle.running.capture.Events()
		}
		select {
		case <-ctx.Done():
			return
		case <-changed:
		case <-retryAt:
		case <-s.wake:
			s.updateMembers(&cycle)
		case event, ok := <-events:
			cycle.captureEvent(ctx, tab, event, ok)
		}
	}
}

// captureCycle bundles the run loop's state for its capture and membership phases.
type captureCycle struct {
	viewers              map[uint64]*streamMember
	running              *runningCapture
	epoch                uint32
	retry                time.Duration
	attempt              time.Time
	unavailable, failure *string
}

func (c *captureCycle) report(reason string) {
	if c.failure != nil && *c.failure == reason {
		return
	}
	c.failure = &reason
	for _, v := range c.viewers {
		v.send(picture{kind: pictureFailed, reason: reason})
	}
}

func (s *captureStream) updateMembers(cycle *captureCycle) {
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
	for id := range cycle.viewers {
		if _, ok := current[id]; !ok {
			delete(cycle.viewers, id)
		}
	}
	key, ack := false, false
	for id, item := range current {
		v := cycle.viewers[id]
		if v == nil {
			v = &streamMember{view: item.view, window: 4, encoding: encoding{fps: 60, scale: 1}}
			cycle.viewers[id] = v
			if cycle.unavailable != nil {
				v.send(picture{kind: pictureUnavailable, reason: *cycle.unavailable})
			}
			if cycle.failure != nil {
				v.send(picture{kind: pictureFailed, reason: *cycle.failure})
			}
			if cycle.running != nil {
				v.floor = cycle.running.sequence
				v.send(
					picture{
						kind:   pictureRestart,
						epoch:  cycle.running.epoch,
						width:  cycle.running.width,
						height: cycle.running.height,
					},
				)
				key = true
			}
		}
		p := item.pace
		if p.paced && cycle.running != nil && p.epoch == cycle.running.epoch {
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
	acknowledgeCapture(cycle, ack, key)
}

func (c *captureCycle) restart(
	ctx context.Context,
	tab *tabs.Tab,
	captures *tabs.CaptureChannel,
	width, height uint32,
	desired encoding,
) {
	if c.running != nil {
		c.running.close(ctx)
		c.running = nil
	}
	capture, err := captures.Start(ctx, tab.TargetID(), width, height, desired.fps, desired.bitrate)
	if err == nil {
		c.epoch++
		c.running = beginCapture(capture, c.epoch, width, height, desired)
		for _, v := range c.viewers {
			v.floor = 0
			v.send(picture{kind: pictureRestart, epoch: c.epoch, width: width, height: height})
		}
	} else {
		var browserErr *cdp.BrowserError
		if errors.As(err, &browserErr) && browserErr.Kind == cdp.KindUnsupportedCapability {
			reason := browserErr.Message
			c.unavailable = &reason
			for _, v := range c.viewers {
				v.send(picture{kind: pictureUnavailable, reason: *c.unavailable})
			}
		} else {
			slog.Warn("live view capture", "tab", tab.ID(), "error", err)
			c.report(err.Error())
			c.retry = min(max(c.retry*2, 500*time.Millisecond), 5*time.Second)
			c.attempt = time.Now().Add(c.retry)
		}
	}
}

func (c *captureCycle) captureEvent(ctx context.Context, tab *tabs.Tab, event tabs.CaptureEvent, ok bool) {
	if !ok {
		c.running.close(ctx)
		c.running = nil
		c.retry = min(max(c.retry*2, 500*time.Millisecond), 5*time.Second)
		c.attempt = time.Now().Add(c.retry)
		return
	}
	switch event := event.(type) {
	case *tabs.CaptureFrame:
		c.retry = 0
		c.failure = nil
		c.running.sequence = event.Frame.Sequence
		for _, v := range c.viewers {
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
		c.report(event.Reason)
		c.running.close(ctx)
		c.running = nil
		c.retry = min(max(c.retry*2, 500*time.Millisecond), 5*time.Second)
		c.attempt = time.Now().Add(c.retry)
	}
}

func desiredEncoding(viewers map[uint64]*streamMember) encoding {
	desired := encoding{fps: 60, scale: 1}
	for _, v := range viewers {
		desired.fps = min(desired.fps, v.encoding.fps)
		desired.scale = min(desired.scale, v.encoding.scale)
		if v.encoding.bitrate != 0 && (desired.bitrate == 0 || v.encoding.bitrate < desired.bitrate) {
			desired.bitrate = v.encoding.bitrate
		}
	}

	return desired
}

func acknowledgeCapture(cycle *captureCycle, ack, key bool) {
	if cycle.running != nil {
		if ack && len(cycle.viewers) > 0 {
			floor, window := ^uint32(0), ^uint32(0)
			for _, v := range cycle.viewers {
				floor = min(floor, v.floor)
				window = min(window, v.window)
			}
			cycle.running.capture.Ack(floor, window)
		}
		if key {
			cycle.running.capture.KeyFrame()
		}
	}
}
