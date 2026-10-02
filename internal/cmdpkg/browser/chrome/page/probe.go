package page

import (
	"context"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// probe describes the observed elements under one viewport point.
func probe(ctx context.Context, tab *tabs.Tab, input browserop.ProbeInput, refs *tabs.References) (browserop.ProbeResult, error) {
	result := browserop.ProbeResult{Viewport: tab.Viewport()}
	p, err := coordinates(ctx, tab, input.XY)
	if err != nil {
		return result, err
	}
	observation, err := captureObservation(ctx, tab.Page(), refs)
	if err != nil {
		return result, err
	}
	var elements []targetElement
	for _, frame := range observation.documents {
		var document *observedDOM
		for i := range observation.dom {
			item := &observation.dom[i]
			if item.frame == frame && item.node.NodeType == 9 {
				document = item
				break
			}
		}
		if document == nil {
			return result, &cdp.BrowserError{Kind: cdp.KindStaleReference}
		}
		key := domIdentity{observation.pages[frame].TargetID(), document.node.BackendNodeID}
		receiver, err := observation.resolveIdentity(ctx, key)
		if err != nil {
			return result, err
		}
		local := p
		for _, owner := range receiver.frames {
			element, err := resolveElement(ctx, owner.page, owner.backend)
			if err != nil {
				return result, err
			}
			offset, err := frameOffset(ctx, element)
			if err != nil {
				return result, err
			}
			local.x -= offset[0]
			local.y -= offset[1]
		}
		keys, err := observation.scanFrame(ctx, frame, scriptProbe, local.x, local.y, browserOption(input.IncludeNonInteractable, false))
		if err != nil {
			return result, err
		}
		for _, key := range keys {
			element, err := observation.resolveIdentity(ctx, key)
			if err != nil {
				return result, err
			}
			elements = append(elements, element)
		}
	}
	elements = observation.domOrder(elements)
	result.Matches, err = observation.describeElements(elements, refs, browserop.DefaultNodes)
	if err != nil {
		return result, err
	}
	for i := range result.Matches {
		element := elements[i]
		bounds, err := decodeElement[[4]float64](ctx, element, "function() { const r = this.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; }", false)
		if err != nil {
			return result, err
		}
		for _, owner := range element.frames {
			frame, err := resolveElement(ctx, owner.page, owner.backend)
			if err != nil {
				return result, err
			}
			offset, err := frameOffset(ctx, frame)
			if err != nil {
				return result, err
			}
			bounds[0] += offset[0]
			bounds[1] += offset[1]
		}
		result.Matches[i].Bounds = &browserop.Bounds{X: bounds[0], Y: bounds[1], Width: bounds[2], Height: bounds[3]}
	}
	result.Truncated = len(elements) > browserop.DefaultNodes
	return result, nil
}
