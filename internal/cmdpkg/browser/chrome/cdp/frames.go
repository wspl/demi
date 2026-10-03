package cdp

import (
	"context"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/dom"
	"github.com/chromedp/cdproto/page"
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
	// Frame holds the captured frame metadata.
	Frame *protocol.Frame
	// Target executes commands in the owning renderer.
	Target FrameTarget
	// ParentTarget identifies the renderer owning the embedding frame element.
	ParentTarget FrameTarget
}

// RendererDocument is the DOM root and frame belonging to one renderer.
type RendererDocument struct {
	// Root holds the renderer's DOM root.
	Root *protocol.Node
	// FrameID identifies the root document frame.
	FrameID protocol.FrameID
}

// FrameSnapshot joins in-process documents and attached renderer sessions.
type FrameSnapshot struct {
	// Main identifies the main document frame.
	Main protocol.FrameID
	// Frames retains captured frames in traversal order.
	Frames []Document
	// Documents indexes each renderer's captured DOM root.
	Documents map[target.ID]RendererDocument
}

// CaptureFrames captures browser frame documents through each owning renderer.
func CaptureFrames(ctx context.Context, renderer FrameTarget) (FrameSnapshot, error) {
	tree, err := page.GetFrameTree().Do(protocol.WithExecutor(ctx, renderer))
	if err != nil {
		return FrameSnapshot{}, err
	}
	snapshot := FrameSnapshot{Main: tree.Frame.ID, Documents: map[target.ID]RendererDocument{}}
	pending := []pendingFrame{{tree, renderer, nil}}
	captured := map[protocol.FrameID]bool{}
	for len(pending) > 0 {
		item := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		if captured[item.tree.Frame.ID] {
			continue
		}
		captured[item.tree.Frame.ID] = true
		owner := item.renderer
		if item.tree.Frame.ID != snapshot.Main {
			related, err := renderer.Related(ctx, target.ID(item.tree.Frame.ID))
			if err != nil {
				return FrameSnapshot{}, err
			}
			if related != nil {
				owner = related
				item.tree, err = page.GetFrameTree().Do(protocol.WithExecutor(ctx, owner))
				if err != nil {
					return FrameSnapshot{}, err
				}
			}
		}
		frame := item.tree.Frame
		if _, exists := snapshot.Documents[owner.TargetID()]; !exists {
			root, err := dom.GetDocument().WithDepth(-1).WithPierce(true).Do(protocol.WithExecutor(ctx, owner))
			if err != nil {
				return FrameSnapshot{}, err
			}
			pending, err = attachedFrames(ctx, renderer, owner, root, captured, pending)
			if err != nil {
				return FrameSnapshot{}, err
			}
			snapshot.Documents[owner.TargetID()] = RendererDocument{Root: root, FrameID: frame.ID}
		}
		for i := len(item.tree.ChildFrames) - 1; i >= 0; i-- {
			pending = append(pending, pendingFrame{item.tree.ChildFrames[i], owner, owner})
		}
		snapshot.Frames = append(snapshot.Frames, Document{Frame: frame, Target: owner, ParentTarget: item.parent})
	}
	return snapshot, nil
}

type pendingFrame struct {
	tree     *page.FrameTree
	renderer FrameTarget
	parent   FrameTarget
}

func attachedFrames(
	ctx context.Context,
	renderer, owner FrameTarget,
	root *protocol.Node,
	captured map[protocol.FrameID]bool,
	pending []pendingFrame,
) ([]pendingFrame, error) {
	nodes := []*protocol.Node{root}
	for len(nodes) > 0 {
		node := nodes[len(nodes)-1]
		nodes = nodes[:len(nodes)-1]
		if node.FrameID != "" && !captured[node.FrameID] {
			child, err := renderer.Related(ctx, target.ID(node.FrameID))
			if err != nil {
				return nil, err
			}
			if child != nil {
				subtree, err := page.GetFrameTree().Do(protocol.WithExecutor(ctx, child))
				if err != nil {
					return nil, err
				}
				pending = append(pending, pendingFrame{subtree, owner, owner})
			}
		}
		if node.ContentDocument != nil {
			nodes = append(nodes, node.ContentDocument)
		}
		nodes = append(nodes, node.Children...)
		nodes = append(nodes, node.ShadowRoots...)
	}

	return pending, nil
}
