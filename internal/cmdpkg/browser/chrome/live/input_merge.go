package live

import "github.com/wspl/demi/internal/cmdpkg/browser/browserop"

// newestInput merges queued continuations, retaining the first ordering boundary.
func newestInput(message browserop.LiveViewerMessage, items <-chan inputItem) (
	browserop.LiveViewerMessage, inputItem, bool,
) {
	for {
		select {
		case item, ok := <-items:
			if !ok {
				return message, inputItem{}, false
			}
			if item.kind != inputMessage {
				return message, item, true
			}
			merged, ok := mergeInput(message, item.message)
			if !ok {
				return message, item, true
			}
			message = merged
		default:
			return message, inputItem{}, false
		}
	}
}

// mergeInput combines wheel turns or replaces pointer moves for the same tab and held modifiers.
func mergeInput(message, next browserop.LiveViewerMessage) (browserop.LiveViewerMessage, bool) {
	if current, ok := message.(*browserop.LiveViewerMessageWheel); ok {
		following, ok := next.(*browserop.LiveViewerMessageWheel)
		if !ok || current.Tab != following.Tab || current.Modifiers != following.Modifiers {
			return nil, false
		}
		merged := *following
		merged.DeltaX += current.DeltaX
		merged.DeltaY += current.DeltaY
		return &merged, true
	}
	if current, ok := message.(*browserop.LiveViewerMessagePointer); ok {
		following, ok := next.(*browserop.LiveViewerMessagePointer)
		if ok && current.Action == browserop.PointerActionMove && following.Action == browserop.PointerActionMove &&
			current.Tab == following.Tab && current.Buttons == following.Buttons && current.Modifiers == following.Modifiers {
			return next, true
		}
	}
	return nil, false
}
