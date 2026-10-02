package cdp

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/target"
)

// FrameTarget supplies a renderer executor and its attached descendants.
// Session implements this interface; page actions can use a scripted target.
type FrameTarget interface {
	Executor
	TargetID() target.ID
	Related(context.Context, target.ID) (FrameTarget, error)
}

// Document identifies a frame and the renderer that owns its DOM.
type Document struct {
	Frame        *protocol.Frame
	Target       FrameTarget
	ParentTarget FrameTarget
}

// RendererDocument is the DOM root and frame belonging to one renderer.
type RendererDocument struct {
	Root    *protocol.Node
	FrameID protocol.FrameID
}

// FrameSnapshot joins in-process documents and attached renderer sessions.
type FrameSnapshot struct {
	Main      protocol.FrameID
	Frames    []Document
	Documents map[target.ID]RendererDocument
}

// CaptureFrames captures browser frame documents through each owning renderer.
func CaptureFrames(ctx context.Context, renderer FrameTarget) (FrameSnapshot, error) {
	panic("not written: k-chrome-cdp")
}
