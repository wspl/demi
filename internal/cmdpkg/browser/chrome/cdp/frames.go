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
	tree, err := page.GetFrameTree().Do(protocol.WithExecutor(ctx, renderer))
	if err != nil {
		return FrameSnapshot{}, err
	}
	snapshot := FrameSnapshot{Main: tree.Frame.ID, Documents: map[target.ID]RendererDocument{}}
	type pendingFrame struct {
		tree     *page.FrameTree
		renderer FrameTarget
		parent   FrameTarget
	}
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
			nodes := []*protocol.Node{root}
			for len(nodes) > 0 {
				node := nodes[len(nodes)-1]
				nodes = nodes[:len(nodes)-1]
				if node.FrameID != "" && !captured[node.FrameID] {
					child, err := renderer.Related(ctx, target.ID(node.FrameID))
					if err != nil {
						return FrameSnapshot{}, err
					}
					if child != nil {
						subtree, err := page.GetFrameTree().Do(protocol.WithExecutor(ctx, child))
						if err != nil {
							return FrameSnapshot{}, err
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
			snapshot.Documents[owner.TargetID()] = RendererDocument{Root: root, FrameID: frame.ID}
		}
		for i := len(item.tree.ChildFrames) - 1; i >= 0; i-- {
			pending = append(pending, pendingFrame{item.tree.ChildFrames[i], owner, owner})
		}
		snapshot.Frames = append(snapshot.Frames, Document{Frame: frame, Target: owner, ParentTarget: item.parent})
	}
	return snapshot, nil
}
