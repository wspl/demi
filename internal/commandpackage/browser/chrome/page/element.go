package page

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/dom"
	"github.com/chromedp/cdproto/runtime"
	"github.com/chromedp/cdproto/target"
	jsonv2 "github.com/go-json-experiment/json"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/cdp"
	"github.com/wspl/demi/internal/commandpackage/browser/chrome/tabs"
)

const objectGroup = "demi-browser-command"

type domIdentity struct {
	target  target.ID
	backend protocol.BackendNodeID
}
type frameOwner struct {
	page    cdp.FrameTarget
	backend protocol.BackendNodeID
}
type targetElement struct {
	page    cdp.FrameTarget
	frames  []frameOwner
	backend protocol.BackendNodeID
	object  runtime.RemoteObjectID
}

// identity keys a DOM node by both renderer and backend identity.
func (e targetElement) identity() domIdentity {
	return domIdentity{e.page.TargetID(), e.backend}
}

// resolveElement binds the observed node to a command-owned remote object.
func resolveElement(ctx context.Context, page cdp.FrameTarget, backend protocol.BackendNodeID) (targetElement, error) {
	object, err := dom.ResolveNode().
		WithBackendNodeID(backend).
		WithObjectGroup(objectGroup).
		Do(protocol.WithExecutor(ctx, page))
	if err != nil {
		return targetElement{}, err
	}
	if object.ObjectID == "" {
		return targetElement{}, &cdp.BrowserError{Kind: cdp.KindStaleReference}
	}
	return targetElement{page: page, backend: backend, object: object.ObjectID}, nil
}

// pageScriptCall preserves empty and null page-algorithm arguments across CDP.
func pageScriptCall(script string, args []any) (*runtime.CallFunctionOnParams, error) {
	// cdproto's CallArgument.Value has json/v2 omitempty: [], "" and null
	// disappear. A nonempty envelope retains those values inside its args field.
	payload, err := cdp.Value(struct {
		Args []any `json:"args"`
	}{Args: args})
	if err != nil {
		return nil, err
	}
	return runtime.CallFunctionOn(fmt.Sprintf("function(input) { return (%s).apply(this, input.args); }", script)).
			WithArguments([]*runtime.CallArgument{{Value: payload}}),
		nil
}

// elementCall runs a page algorithm with the selected node as its receiver.
func elementCall(
	ctx context.Context,
	element targetElement,
	script string,
	gesture bool,
	args ...any,
) (json.RawMessage, error) {
	call, err := pageScriptCall(script, args)
	if err != nil {
		return nil, err
	}
	object, exception, err := call.WithObjectID(element.object).
		WithAwaitPromise(true).
		WithUserGesture(gesture).
		WithReturnByValue(true).
		Do(protocol.WithExecutor(ctx, element.page))
	if err != nil {
		return nil, err
	}
	if err = evaluationException(exception); err != nil {
		return nil, err
	}
	if object != nil {
		if object.Value != nil {
			return json.RawMessage(object.Value), nil
		}
		if object.Subtype == runtime.SubtypeNull || object.Type == runtime.TypeUndefined {
			return json.RawMessage("null"), nil
		}
	}
	return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "element call returned no JSON value"}
}

// decodeElement validates the CDP algorithm's payload before page logic consumes it.
func decodeElement[T any](
	ctx context.Context,
	element targetElement,
	script string,
	gesture bool,
	args ...any,
) (T, error) {
	var result T
	raw, err := elementCall(ctx, element, script, gesture, args...)
	if err != nil {
		return result, err
	}
	return decodePageValue[T](raw)
}

type elementState struct {
	// FillKind identifies the supported input replacement method.
	FillKind string `json:"fillKind"`
	// Attached reports whether the element remains in the document.
	Attached bool `json:"attached"`
	// Visible reports whether the element is rendered.
	Visible bool `json:"visible"`
	// Enabled reports whether the control accepts input.
	Enabled bool `json:"enabled"`
	// Checked holds the checkbox or radio state when applicable.
	Checked *bool `json:"checked"`
	// Radio reports whether the control is a radio button.
	Radio bool `json:"radio"`
	// Failed names the unmet readiness condition.
	Failed *string `json:"failed"`
	// Interceptor describes the element covering the input point.
	Interceptor *string `json:"interceptor"`
	// Permanent reports whether resampling cannot satisfy the condition.
	Permanent bool `json:"permanent"`
	// X holds the horizontal input coordinate.
	X float64 `json:"x"`
	// Y holds the vertical input coordinate.
	Y float64 `json:"y"`
}

// failure retains the condition and intercepting element from the shared algorithm.
func (s elementState) failure() error {
	condition := "attached"
	if s.Failed != nil {
		condition = *s.Failed
	}
	return &cdp.BrowserError{
		Kind:    cdp.KindNotActionable,
		Details: browserproto.ErrorDetails{Condition: &condition, Interceptor: s.Interceptor},
	}
}

// needsCheck rejects inapplicable radio and checkbox mutations before input.
func (s elementState) needsCheck(desired bool) (bool, error) {
	if s.Radio && !desired {
		return false, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "a radio cannot be unchecked"}
	}
	if s.Checked == nil {
		condition := "checkable"
		return false, &cdp.BrowserError{
			Kind:    cdp.KindNotActionable,
			Details: browserproto.ErrorDetails{Condition: &condition},
		}
	}
	return *s.Checked != desired, nil
}

// state reads element readiness including its embedding frames.
func state(ctx context.Context, element targetElement, conditions []string, scroll bool) (elementState, error) {
	result, err := decodeElement[elementState](ctx, element, scriptElementState, false, conditions, scroll)
	if err != nil {
		return result, err
	}
	for _, owner := range element.frames {
		frame, err := resolveElement(ctx, owner.page, owner.backend)
		if err != nil {
			return result, err
		}
		local, err := decodeElement[elementState](ctx, frame, scriptElementState, false, []string{}, false)
		if err != nil {
			return result, err
		}
		result.Visible = result.Visible && local.Visible
		result.Enabled = result.Enabled && local.Enabled
	}
	if result.Failed == nil {
		for _, condition := range conditions {
			if condition == "visible" && !result.Visible || condition == "enabled" && !result.Enabled {
				result.Failed = &condition
				break
			}
		}
	}
	return result, nil
}

// single selects the sole visible candidate when the locator has several matches;
// the bool is false when there is no candidate.
func single(ctx context.Context, elements []targetElement) (targetElement, bool, error) {
	if len(elements) == 0 {
		return targetElement{}, false, nil
	}
	if len(elements) == 1 {
		return elements[0], true, nil
	}
	var visible []targetElement
	for _, element := range elements {
		s, err := state(ctx, element, []string{}, false)
		if err != nil {
			return targetElement{}, false, err
		}
		if s.Visible {
			visible = append(visible, element)
		}
	}
	if len(visible) == 1 {
		return visible[0], true, nil
	}
	return targetElement{}, false, &cdp.BrowserError{Kind: cdp.KindAmbiguous, Count: uint(len(elements))}
}

// frameOffset translates child-document coordinates through its embedding element.
func frameOffset(ctx context.Context, element targetElement) ([2]float64, error) {
	return decodeElement[[2]float64](
		ctx,
		element,
		"function() { const box = this.getBoundingClientRect(); return [box.left + this.clientLeft, "+
			"box.top + this.clientTop]; }",
		false,
	)
}

// localPreparedState joins the page's animation probe cleanup after cancellation.
func localPreparedState(ctx context.Context, element targetElement, conditions []string) (elementState, error) {
	probe, err := cdp.Fresh("probe")
	if err != nil {
		return elementState{}, err
	}
	result, err := decodeElement[elementState](ctx, element, scriptElementState, false, conditions, false, probe, false)
	if err != nil {
		stable := false
		for _, condition := range conditions {
			stable = stable || condition == "stable"
		}
		if stable {
			cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
			defer cancel()
			_, releaseErr := elementCall(cleanup, element, scriptElementState, false, []string{}, false, probe, true)
			var failure *cdp.BrowserError
			if errors.As(releaseErr, &failure) &&
				(failure.Kind == cdp.KindConnection || failure.Kind == cdp.KindClosed) {
				releaseErr = nil
			}
			err = cdp.AfterCleanup(err, releaseErr)
		}
	}
	return result, err
}

// preparedState scrolls all frames before stability and translates hit-test points.
func preparedState(ctx context.Context, element targetElement, conditions []string, scroll bool) (elementState, error) {
	frames := make([]targetElement, 0, len(element.frames))
	for _, owner := range element.frames {
		frame, err := resolveElement(ctx, owner.page, owner.backend)
		if err != nil {
			return elementState{}, err
		}
		frames = append(frames, frame)
	}
	if scroll {
		if err := scrollFrames(ctx, element, frames); err != nil {
			return elementState{}, err
		}
	}
	geometry, hit := false, false
	for _, condition := range conditions {
		geometry = geometry || condition == "geometry"
		hit = hit || condition == "hit"
	}
	if scroll && len(frames) > 0 && geometry {
		if err := tabs.Paint(ctx, frames[len(frames)-1].page); err != nil {
			return elementState{}, err
		}
	}
	for i := len(frames) - 1; i >= 0; i-- {
		frameConditions := []string{"geometry"}
		for _, condition := range conditions {
			if condition == "visible" || condition == "enabled" || condition == "stable" {
				frameConditions = append(frameConditions, condition)
			}
		}
		s, err := localPreparedState(ctx, frames[i], frameConditions)
		if err != nil || s.Failed != nil {
			return s, err
		}
	}
	result, err := localPreparedState(ctx, element, conditions)
	if err != nil || result.Failed != nil {
		return result, err
	}
	return translateFramePoint(ctx, frames, hit, result)
}

// callWithStates supplies the shared readiness algorithm to form algorithms.
func callWithStates[T any](ctx context.Context, element targetElement, script string, args ...any) (T, error) {
	return decodeElement[T](
		ctx,
		element,
		fmt.Sprintf(
			"async function(...args) { const elementState = (%s); return (%s).apply(this, args); }",
			scriptElementState,
			script,
		),
		false,
		args...)
}

// resampleWait waits 50 ms between locator retries within the operation.
func resampleWait(ctx context.Context) error {
	timer := time.NewTimer(50 * time.Millisecond)
	defer timer.Stop()
	select {
	case <-timer.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

type readyElement struct {
	element targetElement
	state   elementState
	named   browserproto.ResolvedElement
}

// ready resamples locators until the selected element satisfies action conditions.
func ready(
	ctx context.Context,
	tab *tabs.Tab,
	target browserproto.BrowserTarget,
	refs *tabs.References,
	conditions []string,
	operation *cdp.Operation,
	last *error,
) (readyElement, error) {
	for {
		var result readyElement
		var found bool
		err := operation.Run(ctx, func(work context.Context) error {
			var err error
			result, found, err = readinessSample(work, tab, target, refs, conditions, last)
			return err
		})
		if err != nil {
			if cdp.IsDeadline(err) {
				cause := *last
				if cause == nil {
					cause = &cdp.BrowserError{Kind: cdp.KindTargetNotFound}
				}
				err = cdp.WithDeadlineCause(err, cause)
			}
			return readyElement{}, err
		}
		if found {
			return result, nil
		}
	}
}

// releaseObjects releases all renderer objects unless a dialog prevents Runtime calls.
func releaseObjects(ctx context.Context, tab *tabs.Tab) error {
	if tab.Dialog().IsOpen() || tab.Context().Err() != nil {
		return nil
	}
	cleanup, cancel := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
	defer cancel()
	snapshot, err := cdp.CaptureFrames(cleanup, tab.Page())
	if err == nil {
		released := map[target.ID]bool{}
		for _, document := range snapshot.Frames {
			id := document.Target.TargetID()
			if released[id] {
				continue
			}
			released[id] = true
			err = runtime.ReleaseObjectGroup(objectGroup).Do(protocol.WithExecutor(cleanup, document.Target))
			if err != nil {
				break
			}
		}
	}
	var failure *cdp.BrowserError
	if errors.As(err, &failure) &&
		(failure.Kind == cdp.KindClosed || failure.Kind == cdp.KindConnection || failure.Kind == cdp.KindTabNotFound) {
		return nil
	}
	return err
}

// UnmarshalJSON validates the browser algorithm's required state fields and enum.
func (s *elementState) UnmarshalJSON(data []byte) error {
	var fields map[string]json.RawMessage
	if err := jsonv2.Unmarshal(data, &fields); err != nil {
		return err
	}
	for _, name := range []string{"fillKind", "attached", "visible", "enabled", "radio", "permanent", "x", "y"} {
		raw, ok := fields[name]
		if !ok {
			return fmt.Errorf("missing field `%s`", name)
		}
		if string(raw) == "null" {
			return fmt.Errorf("invalid null for `%s`", name)
		}
	}
	type stateRecord elementState
	var next stateRecord
	if err := jsonv2.Unmarshal(data, &next, jsonv2.RejectUnknownMembers(true)); err != nil {
		return err
	}
	switch next.FillKind {
	case "none", "native", "text":
	default:
		return fmt.Errorf("unknown variant `%s`, expected one of `none`, `native`, `text`", next.FillKind)
	}
	*s = elementState(next)
	return nil
}

func scrollFrames(ctx context.Context, element targetElement, frames []targetElement) error {
	for i := len(frames) - 1; i >= 0; i-- {
		if _, err := decodeElement[elementState](
			ctx,
			frames[i],
			scriptElementState,
			false,
			[]string{},
			true,
		); err != nil {
			return err
		}
	}
	if _, err := decodeElement[elementState](
		ctx,
		element,
		scriptElementState,
		false,
		[]string{},
		true,
	); err != nil {
		return err
	}

	return nil
}

func translateFramePoint(
	ctx context.Context,
	frames []targetElement,
	hit bool,
	result elementState,
) (elementState, error) {
	for _, frame := range frames {
		offset, err := frameOffset(ctx, frame)
		if err != nil {
			return result, err
		}
		result.X += offset[0]
		result.Y += offset[1]
		if hit {
			s, err := decodeElement[elementState](
				ctx,
				frame,
				scriptElementState,
				false,
				[]string{"visible", "geometry", "hit"},
				false,
				nil,
				false,
				[2]float64{result.X, result.Y},
			)
			if err != nil || s.Failed != nil {
				return s, err
			}
		}
	}
	return result, nil
}

func readinessSample(
	work context.Context,
	tab *tabs.Tab,
	target browserproto.BrowserTarget,
	refs *tabs.References,
	conditions []string,
	last *error,
) (readyElement, bool, error) {
	observation, err := captureObservation(work, tab.Page(), refs)
	if err != nil {
		return readyElement{}, false, err
	}
	elements, err := observation.resolve(work, tab.Page(), target, refs)
	if err != nil {
		var failure *cdp.BrowserError
		if !errors.As(err, &failure) || failure.Kind != cdp.KindStaleReference || !canResample(target) {
			return readyElement{}, false, err
		}
		elements = nil
	}
	element, found, err := single(work, elements)
	if err != nil {
		return readyElement{}, false, err
	}
	if !found {
		return readyElement{}, false, resampleWait(work)
	}
	scroll := false
	condition := "attached"
	for _, c := range conditions {
		scroll = scroll || c == "visible" || c == "geometry"
		if c == "stable" {
			condition = "stable"
		}
	}
	if *last == nil {
		*last = &cdp.BrowserError{
			Kind:    cdp.KindNotActionable,
			Details: browserproto.ErrorDetails{Condition: &condition},
		}
	}
	s, err := preparedState(work, element, conditions, scroll)
	if err != nil {
		return readyElement{}, false, err
	}
	if s.Failed == nil {
		named, err := observation.named(element, refs)
		if err != nil {
			return readyElement{}, false, err
		}
		return readyElement{element, s, named}, true, nil
	}
	if s.Permanent {
		return readyElement{}, false, s.failure()
	}
	*last = s.failure()
	return readyElement{}, false, resampleWait(work)
}
