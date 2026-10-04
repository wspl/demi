package page

import (
	"context"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

// query resolves branches against a single browser observation and scope.
func (o *observation) query(
	ctx context.Context,
	page cdp.FrameTarget,
	q browserproto.BrowserQuery,
	refs *tabs.References,
	scope *domIdentity,
) ([]targetElement, error) {
	var err error
	scope, err = o.queryScope(ctx, page, q, refs, scope)
	if err != nil {
		return nil, err
	}
	var matches []targetElement
	if q.Match != nil {
		matches, err = o.resolve(ctx, page, browserproto.BrowserTarget{BrowserQueryMatch: *q.Match}, refs)
	} else {
		matches, err = o.queryBranches(ctx, page, q, refs, scope)
	}
	if err != nil {
		return nil, err
	}
	if scope != nil {
		matches = slices.DeleteFunc(
			matches,
			func(element targetElement) bool {
				return !o.descendant(element.identity(), *scope, false)
			},
		)
	}
	matches, err = o.filterHas(ctx, page, q, refs, matches)
	if err != nil {
		return nil, err
	}
	matches, err = filterQueryState(ctx, q, matches)
	if err != nil {
		return nil, err
	}
	matches = o.domOrder(matches)
	if q.Nth != nil {
		if *q.Nth >= uint(len(matches)) {
			return []targetElement{}, nil
		}
		matches = matches[*q.Nth : *q.Nth+1]
	}
	return matches, nil
}

// filterHas keeps the matches that have a descendant matching q.Has, and
// none matching q.HasNot.
func (o *observation) filterHas(
	ctx context.Context,
	page cdp.FrameTarget,
	q browserproto.BrowserQuery,
	refs *tabs.References,
	matches []targetElement,
) ([]targetElement, error) {
	for i, filter := range []*browserproto.BrowserQuery{q.Has, q.HasNot} {
		if filter == nil {
			continue
		}
		var filtered []targetElement
		for _, element := range matches {
			key := element.identity()
			descendants, err := o.query(ctx, page, *filter, refs, &key)
			if err != nil {
				return nil, err
			}
			if (len(descendants) > 0) != (i == 1) {
				filtered = append(filtered, element)
			}
		}
		matches = filtered
	}
	return matches, nil
}

func (o *observation) queryScope(
	ctx context.Context,
	page cdp.FrameTarget,
	q browserproto.BrowserQuery,
	refs *tabs.References,
	scope *domIdentity,
) (*domIdentity, error) {
	for i, container := range []*browserproto.BrowserQuery{q.Frame, q.Within} {
		if container == nil {
			continue
		}
		elements, err := o.query(ctx, page, *container, refs, scope)
		if err != nil {
			return nil, err
		}
		element, found, err := single(ctx, elements)
		if err != nil {
			return nil, err
		}
		if !found {
			return nil, &cdp.BrowserError{Kind: cdp.KindTargetNotFound}
		}
		if i == 0 {
			frame, err := decodeElement[bool](
				ctx,
				element,
				"function() { return this.localName === 'iframe' || this.localName === 'frame'; }",
				false,
			)
			if err != nil {
				return nil, err
			}
			if !frame {
				return nil, &cdp.BrowserError{
					Kind:    cdp.KindConfiguration,
					Message: "query frame scope must resolve to a frame",
				}
			}
		}
		key := element.identity()
		scope = &key
	}

	return scope, nil
}

func (o *observation) queryBranches(
	ctx context.Context,
	page cdp.FrameTarget,
	q browserproto.BrowserQuery,
	refs *tabs.References,
	scope *domIdentity,
) ([]targetElement, error) {
	var matches []targetElement
	branches := q.And
	if branches == nil {
		branches = q.Or
	}
	for i, branch := range *branches {
		next, err := o.query(ctx, page, branch, refs, scope)
		if err != nil {
			return nil, err
		}
		if i == 0 || q.And == nil {
			matches = append(matches, next...)
		} else {
			ids := map[domIdentity]bool{}
			for _, element := range next {
				ids[element.identity()] = true
			}
			matches = slices.DeleteFunc(
				matches,
				func(element targetElement) bool {
					return !ids[element.identity()]
				},
			)
		}
	}
	return matches, nil
}

func filterQueryState(
	ctx context.Context,
	q browserproto.BrowserQuery,
	matches []targetElement,
) ([]targetElement, error) {
	if q.HasText == nil && q.HasNotText == nil && q.Visible == nil {
		return matches, nil
	}
	var filtered []targetElement
	for _, element := range matches {
		if q.Visible != nil {
			s, err := state(ctx, element, []string{}, false)
			if err != nil {
				return nil, err
			}
			if s.Visible != *q.Visible {
				continue
			}
		}
		if q.HasText != nil || q.HasNotText != nil {
			text, err := decodeElement[string](ctx, element, "function() { return this.innerText || ''; }", false)
			if err != nil {
				return nil, err
			}
			if q.HasText != nil && !strings.Contains(text, *q.HasText) ||
				q.HasNotText != nil && strings.Contains(text, *q.HasNotText) {
				continue
			}
		}
		filtered = append(filtered, element)
	}
	matches = filtered

	return matches, nil
}
