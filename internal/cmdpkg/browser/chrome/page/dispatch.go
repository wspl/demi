package page

import (
	"context"
	"encoding/json"
	"errors"
	"math"
	"unicode/utf8"

	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/dom"
	chrome "github.com/chromedp/cdproto/page"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// browserOption applies the browser contract's default to an omitted option.
func browserOption[T any](value *T, fallback T) T {
	if value != nil {
		return *value
	}
	return fallback
}

// dispatch runs one admitted browser command over tab-owned state.
func dispatch(
	ctx context.Context,
	tab *tabs.Tab,
	command browserproto.Operation,
	refs *tabs.References,
	operation *cdp.Operation,
	navigation *tabs.NavigationObservation,
) (any, error) {
	switch input := command.(type) {
	case *browserproto.InfoInput:
		return Metadata(ctx, tab, input.Timeout())
	case *browserproto.GotoInput:
		return navigateResult(
			ctx,
			tab,
			&tabs.Visit{URL: input.URL},
			browserOption(input.Load, browserproto.Load("domcontentloaded")),
			operation,
			refs,
		)
	case *browserproto.ReloadInput:
		return navigateResult(
			ctx,
			tab,
			&tabs.Reload{},
			browserOption(input.Load, browserproto.Load("domcontentloaded")),
			operation,
			refs,
		)
	case *browserproto.BackInput:
		return historyNavigation(ctx, tab, true, input.Load, operation, refs)
	case *browserproto.ForwardInput:
		return historyNavigation(ctx, tab, false, input.Load, operation, refs)
	case *browserproto.HistoryInput:
		return readHistory(ctx, tab, input)
	case *browserproto.InspectInput:
		return inspect(ctx, tab, input, refs)
	case *browserproto.FindInput:
		return find(ctx, tab, input, refs)
	case *browserproto.ReadInput:
		return readElements(ctx, tab, *input, refs)
	case *browserproto.EvalInput:
		return evaluate(ctx, tab, input, refs)
	case *browserproto.LogsInput:
		return tab.Console().Read(ctx, *input)
	default:
		return dispatchControls(ctx, tab, command, refs, operation, navigation)
	}
}

// historyNavigation selects and commits the adjacent browser history entry.
func historyNavigation(
	ctx context.Context,
	tab *tabs.Tab,
	back bool,
	load *browserproto.Load,
	operation *cdp.Operation,
	refs *tabs.References,
) (browserproto.NavigationResult, error) {
	loadState := browserOption(load, browserproto.Load("domcontentloaded"))
	entry, err := tab.HistoryStep(ctx, back, operation)
	if err != nil {
		return browserproto.NavigationResult{}, err
	}
	return navigateResult(ctx, tab, &tabs.History{EntryID: entry.ID}, loadState, operation, refs)
}

// answerDialog answers the exact observed dialog; a subsequent one stays open.
func answerDialog(ctx context.Context, tab *tabs.Tab, accept bool, text *string) (browserproto.DialogResult, error) {
	dialog := tab.Dialog().Open()
	if dialog == nil {
		return browserproto.DialogResult{}, &cdp.BrowserError{Kind: cdp.KindDialogNotFound}
	}
	if accept && (dialog.Type == chrome.DialogTypeAlert || (text != nil && dialog.Type != chrome.DialogTypePrompt)) {
		return browserproto.DialogResult{}, &cdp.BrowserError{Kind: cdp.KindInvalidDialogAction}
	}
	outcome := browserproto.DialogOutcome("dismissed")
	if accept {
		outcome = "accepted"
	}
	err := tab.Dialog().Answer(ctx, dialog, accept, text)
	return browserproto.DialogResult{Type: tabs.DialogType(dialog.Type), Outcome: outcome}, err
}

// textBoundary truncates browser content without splitting a UTF-8 character.
func textBoundary(text string, limit int) int {
	end := min(len(text), limit)
	for end > 0 && end < len(text) && !utf8.RuneStart(text[end]) {
		end--
	}
	return end
}

// readContent shares content extraction between reads and temporary fetch tabs.
func readContent(
	ctx context.Context,
	tab *tabs.Tab,
	format browserproto.ContentFormat,
	refs *tabs.References,
) (string, error) {
	if format == "dom" {
		observation, err := captureObservation(ctx, tab.Page(), refs)
		if err != nil {
			return "", err
		}
		tree, _, err := observation.domTree(refs, math.MaxUint)
		if err != nil {
			return "", err
		}
		raw, err := cdp.Value(tree)
		return string(raw), err
	}
	selector, property := "body", "innerText"
	if format == "html" {
		selector, property = "html", "outerHTML"
	}
	executor := protocol.WithExecutor(ctx, tab.Page())
	document, err := dom.GetDocument().Do(executor)
	if err != nil {
		return "", err
	}
	node, err := dom.QuerySelector(document.NodeID, selector).Do(executor)
	if err != nil {
		return "", err
	}
	if node == 0 {
		return "", &cdp.BrowserError{Kind: cdp.KindTargetNotFound}
	}
	object, err := dom.ResolveNode().WithNodeID(node).WithObjectGroup(objectGroup).Do(executor)
	if err != nil {
		return "", err
	}
	if object == nil || object.ObjectID == "" {
		return "", &cdp.BrowserError{Kind: cdp.KindStaleReference}
	}
	element := targetElement{page: tab.Page(), object: object.ObjectID}
	return decodeElement[string](ctx, element, "function(property) { return this[property] ?? ''; }", false, property)
}

// readElements exposes only the selected property and protects native password values.
func readElements(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.ReadInput,
	refs *tabs.References,
) (browserproto.ReadResult, error) {
	observation, err := captureObservation(ctx, tab.Page(), refs)
	if err != nil {
		return nil, err
	}
	elements, err := observation.resolve(ctx, tab.Page(), input.BrowserTarget, refs)
	if err != nil {
		return nil, err
	}
	all := browserOption(input.All, false)
	if !all && len(elements) == 0 {
		return nil, &cdp.BrowserError{Kind: cdp.KindTargetNotFound}
	}
	if !all && len(elements) != 1 {
		return nil, &cdp.BrowserError{Kind: cdp.KindAmbiguous, Count: uint(len(elements))}
	}
	if (input.Property != nil) == (input.Attribute != nil) {
		return nil, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "read requires exactly one --property or --attribute",
		}
	}
	values := []json.RawMessage{}
	for _, element := range elements[:min(len(elements), browserproto.MaxNodes)] {
		value, err := readElement(ctx, observation, element, input)
		if err != nil {
			return nil, err
		}
		values = append(values, value)
	}
	if all {
		return &browserproto.ReadResultAll{Values: values, Truncated: len(elements) > browserproto.MaxNodes}, nil
	}
	return &browserproto.ReadResultOne{Value: values[0]}, nil
}

// waitCondition waits for exactly one URL, load or element state condition.
func waitCondition(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.WaitInput,
	refs *tabs.References,
) (browserproto.WaitResult, error) {
	count := 0
	if input.URL != nil {
		count++
	}
	if input.Load != nil {
		count++
	}
	if input.State != nil || hasTargetFlags(&input.BrowserTarget) {
		count++
	}
	if count != 1 {
		return browserproto.WaitResult{}, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "wait requires exactly one URL, current-document load, or element condition",
		}
	}
	result := browserproto.WaitResult{Matched: true}
	if input.URL != nil {
		result.Condition = *input.URL
		observer, err := tab.ObserveNavigation(ctx)
		if err != nil {
			return result, err
		}
		defer observer.Close()
		err = observer.WaitURL(ctx, tab.Page(), *input.URL)
		url := observer.URL()
		result.URL = &url
		return result, err
	}
	if input.Load != nil {
		result.Condition = string(*input.Load)
		return result, tab.WaitCurrentLoad(ctx, *input.Load)
	}
	return waitElements(ctx, tab, input, refs, result)
}

func readHistory(
	ctx context.Context,
	tab *tabs.Tab,
	input *browserproto.HistoryInput,
) (any, error) {
	current, entries, err := chrome.GetNavigationHistory().Do(protocol.WithExecutor(ctx, tab.Page()))
	if err != nil {
		return nil, err
	}
	offset, limit := browserOption(input.Offset, uint(0)), browserOption(input.Limit, uint(browserproto.DefaultNodes))
	result := browserproto.HistoryResult{
		Entries:   []browserproto.HistoryEntry{},
		Truncated: uint(len(entries)) > offset+limit,
	}
	for i := min(offset, uint(len(entries))); i < min(offset+limit, uint(len(entries))); i++ {
		entry := entries[i]
		result.Entries = append(
			result.Entries,
			browserproto.HistoryEntry{Index: i, URL: entry.URL, Title: entry.Title, Current: int64(i) == current},
		)
	}
	return result, nil
}

func inspect(ctx context.Context, tab *tabs.Tab, input *browserproto.InspectInput, refs *tabs.References) (any, error) {
	observation, err := captureObservation(ctx, tab.Page(), refs)
	if err != nil {
		return nil, err
	}
	observation.scope, err = observation.targetScope(input.Within, input.Frame, refs)
	if err != nil {
		return nil, err
	}
	view := browserOption(input.View, browserproto.InspectView("accessibility"))
	limit := browserOption(input.Limit, uint(browserproto.DefaultNodes))
	var tree []browserproto.BrowserTreeNode
	var truncated bool
	if view == "dom" {
		tree, truncated, err = observation.domTree(refs, limit)
	} else {
		var nodes []browserproto.BrowserNode
		nodes, truncated, err = observation.tree(refs, limit)
		tree = hierarchy(nodes)
	}
	if err != nil {
		return nil, err
	}
	url, title, err := targetInfo(ctx, tab)
	return browserproto.InspectResult{
		Tab:       tab.ID(),
		URL:       url,
		Title:     title,
		View:      view,
		Tree:      tree,
		Truncated: truncated,
	}, err
}

func find(ctx context.Context, tab *tabs.Tab, input *browserproto.FindInput, refs *tabs.References) (any, error) {
	observation, err := captureObservation(ctx, tab.Page(), refs)
	if err != nil {
		return nil, err
	}
	var elements []targetElement
	if browserOption(input.Query, false) {
		query, parseErr := findQuery(input)
		if parseErr != nil {
			return nil, parseErr
		}
		elements, err = observation.query(ctx, tab.Page(), query, refs, nil)
	} else {
		if input.Body != nil {
			return nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: "find stdin requires --query"}
		}
		elements, err = observation.resolve(ctx, tab.Page(), input.BrowserTarget, refs)
	}
	if err != nil {
		return nil, err
	}
	limit, offset := browserOption(
		input.Limit,
		uint(browserproto.DefaultNodes),
	), min(
		browserOption(input.Offset, uint(0)),
		uint(len(elements)),
	)
	nodes, err := observation.describeElements(elements[offset:], refs, limit)
	return browserproto.FindResult{
		Matches:   nodes,
		Count:     uint(len(elements)),
		Truncated: uint(len(elements)) > offset+limit,
	}, err
}

func evaluate(ctx context.Context, tab *tabs.Tab, input *browserproto.EvalInput, refs *tabs.References) (any, error) {
	if hasTargetFlags(&input.BrowserTarget) {
		observation, err := captureObservation(ctx, tab.Page(), refs)
		if err != nil {
			return nil, err
		}
		elements, err := observation.resolve(ctx, tab.Page(), input.BrowserTarget, refs)
		if err != nil {
			return nil, err
		}
		if len(elements) > 1 && !browserOption(input.All, false) {
			return nil, &cdp.BrowserError{Kind: cdp.KindAmbiguous, Count: uint(len(elements))}
		}
		value, err := targetedEvaluation(ctx, input.Expression, elements, browserOption(input.All, false))
		return browserproto.EvalResult{Value: value}, err
	}
	if browserOption(input.All, false) {
		return nil, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "eval --all requires an element locator",
		}
	}
	value, err := readOnly(ctx, tab.Page(), input.Expression)
	return browserproto.EvalResult{Value: value}, err
}

func contentRead(
	ctx context.Context,
	tab *tabs.Tab,
	input *browserproto.ContentReadInput,
	refs *tabs.References,
) (any, error) {
	format := browserOption(input.Format, browserproto.ContentFormat("text"))
	content, err := readContent(ctx, tab, format, refs)
	if err != nil {
		return nil, err
	}
	truncated := input.Output == nil && len(content) > browserproto.InlineBytes
	if input.Output == nil {
		content = content[:textBoundary(content, browserproto.InlineBytes)]
	}
	url, title, err := targetInfo(ctx, tab)
	return &browserproto.ContentReadResultInline{
		URL:       url,
		Title:     title,
		Format:    format,
		Content:   content,
		Truncated: truncated,
	}, err
}

func findQuery(input *browserproto.FindInput) (browserproto.BrowserQuery, error) {
	if hasTargetFlags(&input.BrowserTarget) {
		return browserproto.BrowserQuery{}, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "find --query cannot combine ordinary target flags",
		}
	}
	if input.Body == nil {
		return browserproto.BrowserQuery{}, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "find --query requires stdin",
		}
	}
	query, parseErr := browserproto.ParseQuery([]byte(*input.Body))
	if parseErr != nil {
		return browserproto.BrowserQuery{}, &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: parseErr.Error(),
			Cause:   parseErr,
		}
	}
	for _, branch := range query.Branches() {
		if branch.Match != nil {
			if err := validateTarget(browserproto.BrowserTarget{BrowserQueryMatch: *branch.Match}); err != nil {
				return browserproto.BrowserQuery{}, err
			}
		}
	}

	return query, nil
}

func readElement(
	ctx context.Context,
	observation *observation,
	element targetElement,
	input browserproto.ReadInput,
) (json.RawMessage, error) {
	var value json.RawMessage
	var err error
	if input.Property == nil {
		value, err = elementCall(
			ctx,
			element,
			"function(name) { return this.getAttribute(name); }",
			false,
			*input.Attribute,
		)
	} else {
		switch *input.Property {
		case "visible", "enabled", "checked":
			var s elementState
			s, err = state(ctx, element, []string{}, false)
			if err == nil {
				switch *input.Property {
				case "visible":
					value, err = cdp.Value(s.Visible)
				case "enabled":
					value, err = cdp.Value(s.Enabled)
				case "checked":
					value, err = cdp.Value(s.Checked)
				}
			}
		default:
			property := string(*input.Property)
			var fallback any
			switch property {
			case "text":
				property = "innerText"
				fallback = ""
			case "html":
				property = "outerHTML"
				fallback = ""
			case "text-content":
				property = "textContent"
			case "value":
				if observation.protected(element.identity()) {
					return nil, &cdp.BrowserError{Kind: cdp.KindProtectedValue}
				}
			}
			value, err = elementCall(
				ctx,
				element,
				"function(property, fallback) { return this[property] ?? fallback; }",
				false,
				property,
				fallback,
			)
		}
	}

	return value, err
}

func waitElements(
	ctx context.Context,
	tab *tabs.Tab,
	input browserproto.WaitInput,
	refs *tabs.References,
	result browserproto.WaitResult,
) (browserproto.WaitResult, error) {
	expected := browserOption(input.State, browserproto.ElementState("visible"))
	result.Condition = string(expected)
	for {
		observation, err := captureObservation(ctx, tab.Page(), refs)
		if err != nil {
			return result, err
		}
		elements, err := observation.resolveWait(ctx, tab.Page(), input.BrowserTarget, refs)
		if err != nil {
			var failure *cdp.BrowserError
			if errors.As(err, &failure) && failure.Kind == cdp.KindStaleReference && canResample(input.BrowserTarget) {
				if err = resampleWait(ctx); err != nil {
					return result, err
				}
				continue
			}
			return result, err
		}
		matched, err := matchesWaitState(ctx, elements, expected)
		if err != nil {
			return result, err
		}
		if matched {
			if len(elements) > 0 {
				named, err := observation.named(elements[0], refs)
				if err != nil {
					return result, err
				}
				result.Target = &named
			}
			return result, nil
		}
		if err := resampleWait(ctx); err != nil {
			return result, err
		}
	}
}

func dispatchControls(
	ctx context.Context,
	tab *tabs.Tab,
	command browserproto.Operation,
	refs *tabs.References,
	operation *cdp.Operation,
	navigation *tabs.NavigationObservation,
) (any, error) {
	switch input := command.(type) {
	case *browserproto.ViewportSetInput:
		viewport := browserproto.BrowserViewport{
			Mode:             "custom",
			Width:            input.Width,
			Height:           input.Height,
			DevicePixelRatio: browserOption(input.Scale, 1.0),
		}
		err := tab.SetViewport(ctx, viewport)
		return browserproto.ViewportResult{Viewport: viewport}, err
	case *browserproto.ViewportResetInput:
		viewport := tab.WebViewport()
		err := tab.SetViewport(ctx, viewport)
		return browserproto.ViewportResult{Viewport: viewport}, err
	case *browserproto.DialogInspectInput:
		result := browserproto.DialogInspectResult{}
		if dialog := tab.Dialog().Open(); dialog != nil {
			result.Dialog = &browserproto.Dialog{Type: tabs.DialogType(dialog.Type), Message: dialog.Message}
		}
		return result, nil
	case *browserproto.DialogAcceptInput:
		return answerDialog(ctx, tab, true, input.Text)
	case *browserproto.DialogDismissInput:
		return answerDialog(ctx, tab, false, nil)
	case *browserproto.ContentReadInput:
		return contentRead(ctx, tab, input, refs)
	case *browserproto.WaitInput:
		return waitCondition(ctx, tab, *input, refs)
	case *browserproto.ProbeInput:
		return probe(ctx, tab, *input, refs)
	default:
		return dispatchInput(ctx, tab, command, refs, operation, navigation)
	}
}

func matchesWaitState(ctx context.Context, elements []targetElement, expected browserproto.ElementState) (bool, error) {
	matched := false
	for _, element := range elements {
		s, err := state(ctx, element, []string{}, false)
		if err != nil {
			return false, err
		}
		switch expected {
		case "visible", "hidden":
			matched = matched || s.Visible
		case "attached", "detached":
			matched = matched || s.Attached
		case "enabled":
			matched = matched || s.Enabled
		}
	}
	if expected == "hidden" || expected == "detached" {
		matched = !matched
	}

	return matched, nil
}

func navigateResult(
	ctx context.Context,
	tab *tabs.Tab,
	navigation tabs.Navigation,
	load browserproto.Load,
	operation *cdp.Operation,
	refs *tabs.References,
) (browserproto.NavigationResult, error) {
	url, err := tab.Navigate(ctx, navigation, load, operation, refs)
	if err != nil {
		return browserproto.NavigationResult{}, err
	}
	return completedNavigation(ctx, tab, url), nil
}
