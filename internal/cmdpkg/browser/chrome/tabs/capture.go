package tabs

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	"github.com/chromedp/cdproto/target"
)

// Frame is one encoded picture received from the capture extension.
// Data is immutable after publication.
type Frame struct {
	Sequence  uint32
	Key       bool
	Timestamp float64
	Width     uint16
	Height    uint16
	Data      []byte
}

// CaptureEvent is a validated event from the extension connection.
//
//sumtype:decl
type CaptureEvent interface{ captureEvent() }

// CaptureStarted reports that the extension started the requested capture.
type CaptureStarted struct{}

func (*CaptureStarted) captureEvent() {}

// CaptureFrame carries one encoded picture.
type CaptureFrame struct{ Frame Frame }

func (*CaptureFrame) captureEvent() {}

// CaptureStalled reports that the page stopped painting before capture began.
type CaptureStalled struct{}

func (*CaptureStalled) captureEvent() {}

// CaptureFailed carries the extension's capture failure reason.
type CaptureFailed struct{ Reason string }

func (*CaptureFailed) captureEvent() {}

// CaptureChannel reaches the environment-owned capture listener and connection.
// Replacement connections retire old captures before accepting replacement work.
type CaptureChannel struct{}

// Start captures a target at the requested encoded dimensions. The extension's
// initial connection has Rust's ten-second bound; failure does not close the tab.
func (c *CaptureChannel) Start(ctx context.Context, targetID target.ID, width, height, fps, bitrate uint32) (*Capture, error) {
	panic("not written: k-chrome-tabs")
}

// Capture owns one capture subscription; its consumer must await Close.
type Capture struct{}

// Next waits for the next event or connection end. Frame payloads are immutable.
func (c *Capture) Next(ctx context.Context) (CaptureEvent, error) {
	panic("not written: k-chrome-tabs")
}

// Ack reports received frames and the maximum in-flight frame window.
// Like Rust, superseding controls are dropped when the control queue is full.
func (c *Capture) Ack(sequence, window uint32) { panic("not written: k-chrome-tabs") }

// KeyFrame requests a key frame through the bounded, nonblocking control queue.
func (c *Capture) KeyFrame() { panic("not written: k-chrome-tabs") }

// Encoding changes bitrate and frame rate through the superseding control queue.
func (c *Capture) Encoding(bitrate, fps uint32) { panic("not written: k-chrome-tabs") }

// Close stops capture and releases its event queue, including on cancellation.
func (c *Capture) Close(ctx context.Context) error { panic("not written: k-chrome-tabs") }
