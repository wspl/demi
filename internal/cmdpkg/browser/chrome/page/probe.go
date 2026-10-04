package page

import (
	"context"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// probe describes the observed elements under one viewport point.
func probe(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ProbeInput,
	refs *tabs.References,
) (browserproto.ProbeResult, error) {
	result := browserproto.ProbeResult{Viewport: tab.Viewport()}
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
		keys, err := observation.probeFrame(ctx, frame, p, input)
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
	result.Matches, err = observation.describeElements(elements, refs, browserproto.DefaultNodes)
	if err != nil {
		return result, err
	}
	if err := probeBounds(ctx, elements, &result); err != nil {
		return result, err
	}
	result.Truncated = len(elements) > browserproto.DefaultNodes
	return result, nil
}

func probeBounds(ctx context.Context, elements []targetElement, result *browserproto.ProbeResult) error {
	for i := range result.Matches {
		element := elements[i]
		bounds, err := decodeElement[[4]float64](
			ctx,
			element,
			"function() { const r = this.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; }",
			false,
		)
		if err != nil {
			return err
		}
		for _, owner := range element.frames {
			frame, err := resolveElement(ctx, owner.page, owner.backend)
			if err != nil {
				return err
			}
			offset, err := frameOffset(ctx, frame)
			if err != nil {
				return err
			}
			bounds[0] += offset[0]
			bounds[1] += offset[1]
		}
		result.Matches[i].Bounds = &browserproto.Bounds{X: bounds[0], Y: bounds[1], Width: bounds[2], Height: bounds[3]}
	}

	return nil
}

func (o *observation) probeFrame(
	ctx context.Context,
	frame protocol.FrameID,
	p point,
	input browserproto.ProbeInput,
) ([]domIdentity, error) {
	var document *observedDOM
	for i := range o.dom {
		item := &o.dom[i]
		if item.frame == frame && item.node.NodeType == 9 {
			document = item
			break
		}
	}
	if document == nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindStaleReference}
	}
	key := domIdentity{o.pages[frame].TargetID(), document.node.BackendNodeID}
	receiver, err := o.resolveIdentity(ctx, key)
	if err != nil {
		return nil, err
	}
	local := p
	for _, owner := range receiver.frames {
		element, err := resolveElement(ctx, owner.page, owner.backend)
		if err != nil {
			return nil, err
		}
		offset, err := frameOffset(ctx, element)
		if err != nil {
			return nil, err
		}
		local.x -= offset[0]
		local.y -= offset[1]
	}
	keys, err := o.scanFrame(
		ctx,
		frame,
		scriptProbe,
		local.x,
		local.y,
		browserOption(input.IncludeNonInteractable, false),
	)
	if err != nil {
		return nil, err
	}

	return keys, nil
}
