package page

import (
	"context"
	"encoding/json"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/dom"
	"github.com/chromedp/cdproto/input"
	chrome "github.com/chromedp/cdproto/page"
	"github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// dispatchInput runs native input with one admission and shared action progress.
func dispatchInput(ctx context.Context, tab *tabs.Tab, command browserop.Operation, refs *tabs.References, operation *cdp.Operation, navigation *tabs.NavigationObservation) (any, error) {
	var named *browserop.ResolvedElement
	var last error
	finish := func(name string, wait *string) (browserop.ActionResult, error) {
		result, err := actionResult(ctx, tab, operation, name, wait, navigation)
		result.Target = named
		return result, err
	}
	switch request := command.(type) {
	case *browserop.ClickInput:
		p, target, err := inputPoint(ctx, tab, request.XY, request.BrowserTarget, refs, []string{"visible", "enabled", "geometry", "stable", "hit"}, operation)
		if err != nil {
			return nil, err
		}
		named = target
		err = clickAt(ctx, tab, p, input.MouseButton(browserOption(request.Button, browserop.MouseButton("left"))), int64(browserOption(request.Count, uint8(1))), pointerModifiers(request.Modifier), operation)
		if err != nil {
			return nil, err
		}
		return finish("click", request.WaitURL)
	case *browserop.MoveInput:
		p, target, err := inputPoint(ctx, tab, request.XY, request.BrowserTarget, refs, []string{"geometry"}, operation)
		if err != nil {
			return nil, err
		}
		named = target
		operation.BeginInput()
		err = input.DispatchMouseEvent(input.MouseMoved, p.x, p.y).WithModifiers(pointerModifiers(request.Modifier)).Do(protocol.WithExecutor(ctx, tab.Page()))
		if err != nil {
			return nil, err
		}
		operation.CompleteInput()
		return finish("move", nil)
	case *browserop.ScrollInput:
		dx, dy := browserOption(request.Dx, 0.0), browserOption(request.Dy, 0.0)
		if dx == 0 && dy == 0 {
			return nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "scroll requires a nonzero dx or dy"}
		}
		var p point
		if request.XY == nil && !hasTargetFlags(&request.BrowserTarget) {
			_, _, _, viewport, _, _, err := chrome.GetLayoutMetrics().Do(protocol.WithExecutor(ctx, tab.Page()))
			if err != nil {
				return nil, err
			}
			p = point{float64(viewport.ClientWidth) / 2, float64(viewport.ClientHeight) / 2}
		} else {
			var err error
			p, named, err = inputPoint(ctx, tab, request.XY, request.BrowserTarget, refs, []string{"geometry"}, operation)
			if err != nil {
				return nil, err
			}
		}
		operation.BeginInput()
		err := input.DispatchMouseEvent(input.MouseWheel, p.x, p.y).WithModifiers(pointerModifiers(request.Modifier)).WithDeltaX(dx).WithDeltaY(dy).Do(protocol.WithExecutor(ctx, tab.Page()))
		if err != nil {
			return nil, err
		}
		operation.CompleteInput()
		return finish("scroll", nil)
	case *browserop.DragInput:
		return drag(ctx, tab, *request, operation)
	case *browserop.FillInput:
		ready, err := ready(ctx, tab, request.BrowserTarget, refs, []string{"fillable", "visible", "enabled", "editable"}, operation, &last)
		if err != nil {
			return nil, err
		}
		named = &ready.named
		native := ready.state.FillKind == "native"
		if native {
			operation.BeginInput()
		}
		mode, err := decodeElement[string](ctx, ready.element, scriptFill, false, request.Text, native)
		if err != nil {
			return nil, err
		}
		switch mode {
		case "invalid":
			operation.InputNotDelivered()
			return nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "browser rejected the native input value"}
		case "native":
			if native {
				operation.CompleteInput()
			}
		case "text":
			if request.Text == "" {
				keys, err := combination("Delete")
				if err != nil {
					return nil, err
				}
				if err = press(ctx, tab, keys, operation); err != nil {
					return nil, err
				}
			} else {
				operation.BeginInput()
				if err = input.InsertText(request.Text).Do(protocol.WithExecutor(ctx, tab.Page())); err != nil {
					return nil, err
				}
				operation.CompleteInput()
			}
		default:
			return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "unknown fill mode"}
		}
		return finish("fill", request.WaitURL)
	case *browserop.TypeInput:
		var element *targetElement
		if hasTargetFlags(&request.BrowserTarget) {
			ready, err := ready(ctx, tab, request.BrowserTarget, refs, []string{"visible", "enabled", "editable"}, operation, &last)
			if err != nil {
				return nil, err
			}
			element = &ready.element
			named = &ready.named
			if err = focus(ctx, *element); err != nil {
				return nil, err
			}
		} else {
			_, target, err := currentFocus(ctx, tab, refs)
			if err != nil {
				return nil, err
			}
			named = target
		}
		if err := typeFocused(ctx, tab, element, request.Text, operation); err != nil {
			return nil, err
		}
		return finish("type", nil)
	case *browserop.KeyInput:
		keys, err := combination(request.Key)
		if err != nil {
			return nil, err
		}
		if hasTargetFlags(&request.BrowserTarget) {
			ready, err := ready(ctx, tab, request.BrowserTarget, refs, []string{"visible", "enabled"}, operation, &last)
			if err != nil {
				return nil, err
			}
			named = &ready.named
			if err = focus(ctx, ready.element); err != nil {
				return nil, err
			}
		} else {
			_, target, err := currentFocus(ctx, tab, refs)
			if err != nil {
				return nil, err
			}
			named = target
		}
		if err = press(ctx, tab, keys, operation); err != nil {
			return nil, err
		}
		return finish("key", request.WaitURL)
	case *browserop.CheckInput:
		checkable, err := ready(ctx, tab, request.BrowserTarget, refs, []string{"checkable"}, operation, &last)
		if err != nil {
			return nil, err
		}
		named = &checkable.named
		needs, err := checkable.state.needsCheck(request.Value)
		if err != nil {
			return nil, err
		}
		if needs {
			ready, err := ready(ctx, tab, request.BrowserTarget, refs, []string{"visible", "enabled", "geometry", "stable", "hit"}, operation, &last)
			if err != nil {
				return nil, err
			}
			needs, err = ready.state.needsCheck(request.Value)
			if err != nil {
				return nil, err
			}
			if needs {
				if err = clickAt(ctx, tab, point{ready.state.X, ready.state.Y}, input.Left, 1, 0, operation); err != nil {
					return nil, err
				}
			}
			s, err := state(ctx, ready.element, []string{}, false)
			if err != nil {
				return nil, err
			}
			if s.Checked == nil || *s.Checked != request.Value {
				condition := "click did not change checked state"
				return nil, &cdp.BrowserError{Kind: cdp.KindNotActionable, Details: browserop.ErrorDetails{Condition: &condition}}
			}
		}
		return finish("check", nil)
	case *browserop.SelectInput:
		return selectOptions(ctx, tab, *request, refs, operation)
	case *browserop.SelectTextInput:
		return selectText(ctx, tab, *request, refs, operation)
	}
	return nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "this operation requires the browser conversation controller"}
}

// inputPoint resolves either coordinates or one actionable element.
func inputPoint(ctx context.Context, tab *tabs.Tab, xy *string, target browserop.BrowserTarget, refs *tabs.References, conditions []string, operation *cdp.Operation) (point, *browserop.ResolvedElement, error) {
	if xy != nil {
		if hasTargetFlags(&target) {
			return point{}, nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "coordinates cannot be combined with an element target"}
		}
		p, err := coordinates(ctx, tab, *xy)
		return p, nil, err
	}
	var last error
	ready, err := ready(ctx, tab, target, refs, conditions, operation, &last)
	if err != nil {
		return point{}, nil, err
	}
	return point{ready.state.X, ready.state.Y}, &ready.named, nil
}

// drag follows the requested points and releases the button even on cancellation.
func drag(ctx context.Context, tab *tabs.Tab, request browserop.DragInput, operation *cdp.Operation) (browserop.ActionResult, error) {
	points := make([]point, 0, len(request.Point))
	for _, value := range request.Point {
		p, err := coordinates(ctx, tab, string(value))
		if err != nil {
			return browserop.ActionResult{}, err
		}
		points = append(points, p)
	}
	first, release := points[0], points[0]
	modifiers := pointerModifiers(request.Modifier)
	err := tab.Input(ctx, operation, func(work context.Context) error {
		operation.BeginInput()
		executor := protocol.WithExecutor(work, tab.Page())
		for i, p := range points {
			release = p
			button, buttons := input.None, int64(0)
			if i > 0 {
				button, buttons = input.Left, 1
			}
			if err := input.DispatchMouseEvent(input.MouseMoved, p.x, p.y).WithButton(button).WithButtons(buttons).WithModifiers(modifiers).Do(executor); err != nil {
				return err
			}
			if i == 0 {
				if err := input.DispatchMouseEvent(input.MousePressed, first.x, first.y).WithButton(input.Left).WithButtons(1).WithClickCount(1).WithModifiers(modifiers).Do(executor); err != nil {
					return err
				}
			}
		}
		return nil
	})
	if err != nil {
		modifiers = 0
	}
	err = releaseMouse(ctx, tab, err, input.DispatchMouseEvent(input.MouseReleased, release.x, release.y).WithButton(input.Left).WithButtons(0).WithModifiers(modifiers).WithClickCount(1))
	if err == nil {
		operation.CompleteInput()
	}
	return browserop.ActionResult{Operation: "drag", Result: json.RawMessage("true")}, err
}

// currentFocus follows frame focus without changing selection or clicking.
func currentFocus(ctx context.Context, tab *tabs.Tab, refs *tabs.References) (targetElement, *browserop.ResolvedElement, error) {
	observation, err := captureObservation(ctx, tab.Page(), refs)
	if err != nil {
		return targetElement{}, nil, err
	}
	page := tab.Page()
	var receiver runtime.RemoteObjectID
	for {
		var object *runtime.RemoteObject
		var exception *runtime.ExceptionDetails
		if receiver == "" {
			object, exception, err = runtime.Evaluate("(" + scriptFocused + ")()").WithObjectGroup(objectGroup).Do(protocol.WithExecutor(ctx, page))
		} else {
			object, exception, err = runtime.CallFunctionOn("function() { return (" + scriptFocused + ")(); }").WithObjectID(receiver).WithObjectGroup(objectGroup).Do(protocol.WithExecutor(ctx, page))
		}
		if err != nil {
			return targetElement{}, nil, err
		}
		if err = evaluationException(exception); err != nil {
			return targetElement{}, nil, err
		}
		if object == nil || object.ObjectID == "" {
			return targetElement{}, nil, &cdp.BrowserError{Kind: cdp.KindStaleReference}
		}
		node, err := dom.DescribeNode().WithObjectID(object.ObjectID).Do(protocol.WithExecutor(ctx, page))
		if err != nil {
			return targetElement{}, nil, err
		}
		if (node.LocalName == "iframe" || node.LocalName == "frame") && node.FrameID != "" {
			next := observation.pages[node.FrameID]
			if next == nil {
				return targetElement{}, nil, &cdp.BrowserError{Kind: cdp.KindStaleReference}
			}
			receiver = ""
			for _, item := range observation.dom {
				if item.frame == node.FrameID && item.node.NodeType == 9 {
					element, err := resolveElement(ctx, next, item.node.BackendNodeID)
					if err != nil {
						return targetElement{}, nil, err
					}
					receiver = element.object
					break
				}
			}
			if receiver == "" {
				return targetElement{}, nil, &cdp.BrowserError{Kind: cdp.KindStaleReference}
			}
			page = next
			continue
		}
		element := targetElement{page: page, backend: node.BackendNodeID, object: object.ObjectID}
		index, accessible := observation.axIndex[element.identity()]
		if node.LocalName == "body" || node.LocalName == "html" || !accessible || observation.nodes[index].ax.Ignored {
			return element, nil, nil
		}
		named, err := observation.named(element, refs)
		return element, &named, err
	}
}
