package live

import "github.com/wspl/demi/internal/cmdpkg/browser/browserproto"

// newestInput merges queued continuations, retaining the first ordering boundary.
func newestInput(message browserproto.LiveViewerMessage, items <-chan inputItem) (
	browserproto.LiveViewerMessage, inputItem, bool,
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
func mergeInput(message, next browserproto.LiveViewerMessage) (browserproto.LiveViewerMessage, bool) {
	if current, ok := message.(*browserproto.LiveViewerMessageWheel); ok {
		following, ok := next.(*browserproto.LiveViewerMessageWheel)
		if !ok || current.Tab != following.Tab || current.Modifiers != following.Modifiers {
			return nil, false
		}
		merged := *following
		merged.DeltaX += current.DeltaX
		merged.DeltaY += current.DeltaY
		return &merged, true
	}
	if current, ok := message.(*browserproto.LiveViewerMessagePointer); ok {
		following, ok := next.(*browserproto.LiveViewerMessagePointer)
		if ok &&
			current.Action == browserproto.PointerActionMove &&
			following.Action == browserproto.PointerActionMove &&
			current.Tab == following.Tab &&
			current.Buttons == following.Buttons &&
			current.Modifiers == following.Modifiers {
			return next, true
		}
	}
	return nil, false
}
