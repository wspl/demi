package page

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"

	"github.com/chromedp/cdproto/accessibility"
	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/chromedp/cdproto/dom"
	"github.com/chromedp/cdproto/runtime"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
)

type observedNode struct {
	ax       *accessibility.Node
	frame    protocol.FrameID
	loader   protocol.LoaderID
	children []int
}
type observedDOM struct {
	node  *protocol.Node
	frame protocol.FrameID
}
type nodeDepth struct {
	index int
	depth uint
}
type observation struct {
	nodes        []observedNode
	order        []nodeDepth
	dom          []observedDOM
	loaders      map[protocol.FrameID]protocol.LoaderID
	domIndex     map[domIdentity]int
	axIndex      map[domIdentity]int
	parents      map[domIdentity]domIdentity
	documents    []protocol.FrameID
	pages        map[protocol.FrameID]cdp.FrameTarget
	inaccessible map[protocol.FrameID]string
	scope        *domIdentity
}

// retainer is the reference state capture needs; issuing and resolving
// references belong to later observation operations, not snapshot acquisition.
type retainer interface {
	Retain(func(tabs.Reference) bool)
}

// captureObservation joins each frame's accessibility tree at its iframe element.
func captureObservation(ctx context.Context, page cdp.FrameTarget, refs retainer) (*observation, error) {
	snapshot, err := cdp.CaptureFrames(ctx, page)
	if err != nil {
		return nil, err
	}
	o := &observation{
		loaders:      map[protocol.FrameID]protocol.LoaderID{},
		domIndex:     map[domIdentity]int{},
		axIndex:      map[domIdentity]int{},
		parents:      map[domIdentity]domIdentity{},
		pages:        map[protocol.FrameID]cdp.FrameTarget{},
		inaccessible: map[protocol.FrameID]string{},
	}
	roots := map[protocol.FrameID]int{}
	type owner struct {
		frame protocol.FrameID
		key   domIdentity
	}
	var owners []owner
	for _, document := range snapshot.Frames {
		frame, renderer := document.Frame, document.Target
		if document.ParentTarget != nil {
			backend, _, err := dom.GetFrameOwner(frame.ID).Do(protocol.WithExecutor(ctx, document.ParentTarget))
			if err != nil {
				return nil, err
			}
			owners = append(owners, owner{frame.ID, domIdentity{document.ParentTarget.TargetID(), backend}})
		}
		o.pages[frame.ID] = renderer
		o.loaders[frame.ID] = frame.LoaderID
		if err := o.captureAX(ctx, frame, renderer, roots); err != nil {
			return nil, err
		}
	}
	refs.Retain(func(reference tabs.Reference) bool {
		loader, ok := o.loaders[reference.Frame]
		return ok && loader == reference.Loader
	})
	for _, owner := range owners {
		if root, ok := roots[owner.frame]; ok {
			if i, ok := o.axIndex[owner.key]; ok {
				o.nodes[i].children = append(o.nodes[i].children, root)
			}
		}
	}
	o.orderAX(roots, snapshot.Main)
	if err := o.captureDOM(page, snapshot); err != nil {
		return nil, err
	}
	return o, nil
}

// axText reads optional Chrome role/name text without inventing a value.
func axText(value *accessibility.Value) string {
	if value == nil {
		return ""
	}
	var text string
	if err := json.Unmarshal(value.Value, &text); err != nil {
		return ""
	} // Non-string AX values have no text.
	return text
}

// protected identifies password inputs before observations or reads expose values.
func (o *observation) protected(key domIdentity) bool {
	i, ok := o.domIndex[key]
	if !ok {
		return false
	}
	node := o.dom[i].node
	if node.LocalName != "input" {
		return false
	}
	for i := 0; i+1 < len(node.Attributes); i += 2 {
		if node.Attributes[i] == "type" && strings.EqualFold(node.Attributes[i+1], "password") {
			return true
		}
	}
	return false
}

// describe retains AX names, scalar values, states and document-bound references.
func (o *observation) describe(index int, depth uint, refs *tabs.References) (browserproto.BrowserNode, error) {
	node := o.nodes[index]
	result := browserproto.BrowserNode{
		Role:   axText(node.ax.Role),
		Name:   axText(node.ax.Name),
		Depth:  depth,
		States: []string{},
	}
	protected := false
	if node.ax.BackendDOMNodeID != 0 {
		id, err := refs.Issue(tabs.Reference{Backend: node.ax.BackendDOMNodeID, Frame: node.frame, Loader: node.loader})
		if err != nil {
			return result, err
		}
		result.Ref = &id
		key := domIdentity{o.pages[node.frame].TargetID(), node.ax.BackendDOMNodeID}
		protected = o.protected(key)
	}
	for _, property := range node.ax.Properties {
		if property.Value == nil || len(property.Value.Value) == 0 {
			continue
		}
		raw := property.Value.Value
		var value any
		if err := json.Unmarshal(raw, &value); err != nil {
			return result, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
		}
		switch value := value.(type) {
		case string:
			result.States = append(result.States, string(property.Name)+"="+value)
		case bool, float64:
			result.States = append(result.States, string(property.Name)+"="+string(raw))
		}
	}
	if node.ax.BackendDOMNodeID != 0 {
		if i, ok := o.domIndex[domIdentity{o.pages[node.frame].TargetID(), node.ax.BackendDOMNodeID}]; ok {
			if _, ok := o.inaccessible[o.dom[i].node.FrameID]; ok {
				result.States = append(result.States, "inaccessible")
			}
		}
	}
	return describeAXValue(node, protected, result)
}

// descendant follows the captured composed DOM across frame boundaries.
func (o *observation) descendant(key, container domIdentity, inclusive bool) bool {
	if !inclusive {
		var ok bool
		key, ok = o.parents[key]
		if !ok {
			return false
		}
	}
	for {
		if key == container {
			return true
		}
		parent, ok := o.parents[key]
		if !ok {
			return false
		}
		key = parent
	}
}

// reference rejects references outside their captured document and renderer.
func (o *observation) reference(id browserproto.NodeRef, refs *tabs.References) (domIdentity, error) {
	ref, ok := refs.Lookup(id)
	if ok && o.loaders[ref.Frame] == ref.Loader && o.pages[ref.Frame] != nil {
		key := domIdentity{o.pages[ref.Frame].TargetID(), ref.Backend}
		if i, ok := o.domIndex[key]; ok && o.dom[i].frame == ref.Frame {
			return key, nil
		}
	}
	return domIdentity{}, &cdp.BrowserError{Kind: cdp.KindStaleReference}
}

// resolveIdentity binds a captured node and retains its embedding frame chain.
func (o *observation) resolveIdentity(ctx context.Context, key domIdentity) (targetElement, error) {
	i, ok := o.domIndex[key]
	if !ok {
		return targetElement{}, &cdp.BrowserError{Kind: cdp.KindStaleReference}
	}
	element, err := resolveElement(ctx, o.pages[o.dom[i].frame], key.backend)
	if err != nil {
		return element, err
	}
	for {
		parent, ok := o.parents[key]
		if !ok {
			break
		}
		i, ok := o.domIndex[parent]
		if !ok {
			return element, &cdp.BrowserError{Kind: cdp.KindStaleReference}
		}
		node := o.dom[i]
		if node.node.LocalName == "iframe" || node.node.LocalName == "frame" {
			element.frames = append(element.frames, frameOwner{o.pages[node.frame], node.node.BackendNodeID})
		}
		key = parent
	}
	return element, nil
}

// targetScope validates ordered frame references and the optional container.
func (o *observation) targetScope(
	within *browserproto.NodeRef,
	frames *[]browserproto.NodeRef,
	refs *tabs.References,
) (*domIdentity, error) {
	var scope *domIdentity
	var expected protocol.FrameID
	if frames != nil {
		for _, ref := range *frames {
			key, child, err := o.frameScope(ref, refs, expected)
			if err != nil {
				return nil, err
			}
			expected = child
			scope = &key
		}
	}
	if within != nil {
		key, err := o.reference(*within, refs)
		if err != nil {
			return nil, err
		}
		if scope != nil && !o.descendant(key, *scope, false) {
			return nil, &cdp.BrowserError{
				Kind:    cdp.KindConfiguration,
				Message: "--within is outside the selected frame",
			}
		}
		scope = &key
	}
	return scope, nil
}

// tree describes the captured accessibility order without losing its hierarchy.
func (o *observation) tree(refs *tabs.References, limit uint) ([]browserproto.BrowserNode, bool, error) {
	nodes := []browserproto.BrowserNode{}
	for _, item := range o.order {
		node := o.nodes[item.index]
		if o.scope != nil &&
			(node.ax.BackendDOMNodeID == 0 || !o.descendant(
				domIdentity{
					o.pages[node.frame].TargetID(),
					node.ax.BackendDOMNodeID,
				},
				*o.scope,
				true,
			)) {
			continue
		}
		if uint(len(nodes)) == limit {
			return nodes, true, nil
		}
		described, err := o.describe(item.index, item.depth, refs)
		if err != nil {
			return nil, false, err
		}
		nodes = append(nodes, described)
	}
	return nodes, false, nil
}

type treeDepth struct {
	depth uint
	node  browserproto.BrowserTreeNode
}

// nest preserves the observed browser hierarchy from preorder depth values.
func nest(nodes []treeDepth) []browserproto.BrowserTreeNode {
	roots := []treeDepth{}
	for i := len(nodes) - 1; i >= 0; i-- {
		node := nodes[i]
		children := []browserproto.BrowserTreeNode{}
		for len(roots) > 0 && roots[len(roots)-1].depth > node.depth {
			children = append(children, roots[len(roots)-1].node)
			roots = roots[:len(roots)-1]
		}
		if len(children) > 0 {
			node.node.Children = &children
		}
		roots = append(roots, node)
	}
	result := make([]browserproto.BrowserTreeNode, 0, len(roots))
	for i := len(roots) - 1; i >= 0; i-- {
		result = append(result, roots[i].node)
	}
	return result
}

// hierarchy converts flat AX descriptions into the inspect tree contract.
func hierarchy(nodes []browserproto.BrowserNode) []browserproto.BrowserTreeNode {
	values := make([]treeDepth, 0, len(nodes))
	for _, node := range nodes {
		values = append(
			values,
			treeDepth{
				node.Depth,
				browserproto.BrowserTreeNode{
					Ref:    node.Ref,
					Role:   &node.Role,
					Name:   &node.Name,
					Value:  node.Value,
					States: &node.States,
				},
			},
		)
	}
	return nest(values)
}

// domTree describes element tags while omitting script and style implementation text.
func (o *observation) domTree(refs *tabs.References, limit uint) ([]browserproto.BrowserTreeNode, bool, error) {
	nodes := []treeDepth{}
	truncated := false
	for _, item := range o.dom {
		node, frame := item.node, item.frame
		key := domIdentity{o.pages[frame].TargetID(), node.BackendNodeID}
		if o.scope != nil && !o.descendant(key, *o.scope, true) {
			continue
		}
		if node.NodeType != 1 || node.LocalName == "script" || node.LocalName == "style" ||
			node.LocalName == "noscript" {
			continue
		}
		if uint(len(nodes)) == limit {
			truncated = true
			break
		}
		depth := uint(0)
		for ancestor := key; ; {
			parent, ok := o.parents[ancestor]
			if !ok {
				break
			}
			if i, ok := o.domIndex[parent]; ok && o.dom[i].node.NodeType == 1 {
				depth++
			}
			ancestor = parent
		}
		id, err := refs.Issue(tabs.Reference{Backend: node.BackendNodeID, Frame: frame, Loader: o.loaders[frame]})
		if err != nil {
			return nil, false, err
		}
		children := []browserproto.BrowserTreeNode{}
		tree := browserproto.BrowserTreeNode{Ref: &id, Tag: &node.LocalName, Children: &children}
		if i, ok := o.axIndex[key]; ok {
			described, err := o.describe(i, 0, refs)
			if err != nil {
				return nil, false, err
			}
			tree.Role, tree.Name, tree.States, tree.Value = &described.Role, &described.Name, &described.States, described.Value
		}
		nodes = append(nodes, treeDepth{depth, tree})
	}
	return nest(nodes), truncated, nil
}

// describeElements names locator matches through the same AX and DOM snapshot.
func (o *observation) describeElements(
	elements []targetElement,
	refs *tabs.References,
	limit uint,
) ([]browserproto.BrowserNode, error) {
	nodes := []browserproto.BrowserNode{}
	for _, element := range elements[:min(uint(len(elements)), limit)] {
		if i, ok := o.axIndex[element.identity()]; ok {
			node, err := o.describe(i, 0, refs)
			if err != nil {
				return nil, err
			}
			nodes = append(nodes, node)
		} else {
			i, ok := o.domIndex[element.identity()]
			if !ok {
				return nil, &cdp.BrowserError{Kind: cdp.KindStaleReference}
			}
			frame := o.dom[i].frame
			id, err := refs.Issue(tabs.Reference{Backend: element.backend, Frame: frame, Loader: o.loaders[frame]})
			if err != nil {
				return nil, err
			}
			nodes = append(nodes, browserproto.BrowserNode{Ref: &id, States: []string{}})
		}
	}
	return nodes, nil
}

// named returns the public identity and semantic description of one element.
func (o *observation) named(element targetElement, refs *tabs.References) (browserproto.ResolvedElement, error) {
	nodes, err := o.describeElements([]targetElement{element}, refs, 1)
	if err != nil {
		return browserproto.ResolvedElement{}, err
	}
	if len(nodes) == 0 || nodes[0].Ref == nil {
		return browserproto.ResolvedElement{}, &cdp.BrowserError{Kind: cdp.KindStaleReference}
	}
	return browserproto.ResolvedElement{Ref: *nodes[0].Ref, Role: nodes[0].Role, Name: nodes[0].Name}, nil
}

// scanFrame resolves composed-tree nodes returned by one page-side algorithm.
func (o *observation) scanFrame(
	ctx context.Context,
	frame protocol.FrameID,
	script string,
	args ...any,
) ([]domIdentity, error) {
	renderer := o.pages[frame]
	var document *protocol.Node
	for _, item := range o.dom {
		if item.frame == frame && item.node.NodeType == 9 {
			document = item.node
			break
		}
	}
	if document == nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindStaleReference}
	}
	receiver, err := resolveElement(ctx, renderer, document.BackendNodeID)
	if err != nil {
		return nil, err
	}
	call, err := pageScriptCall(script, args)
	if err != nil {
		return nil, err
	}
	object, exception, err := call.WithObjectID(receiver.object).
		WithObjectGroup(objectGroup).
		WithSerializationOptions(&runtime.SerializationOptions{
			Serialization: runtime.SerializationOptionsSerializationDeep,
			MaxDepth:      2,
			AdditionalParameters: json.RawMessage(
				`{"maxNodeDepth":0}`,
			),
		}).
		Do(protocol.WithExecutor(ctx, renderer))
	if err != nil {
		return nil, err
	}
	if err = evaluationException(exception); err != nil {
		return nil, err
	}
	if object == nil || object.DeepSerializedValue == nil || object.DeepSerializedValue.Type != "array" {
		return nil, &cdp.BrowserError{
			Kind:    cdp.KindInvalidResult,
			Message: "locator did not return a serialized node array",
		}
	}
	return o.scanNodes(renderer, object)
}

// patternMatches uses Chrome's ECMAScript engine for locator patterns.
func patternMatches(ctx context.Context, page cdp.Executor, pattern string, values []string) ([]bool, error) {
	p, err := cdp.Value(pattern)
	if err != nil {
		return nil, err
	}
	v, err := cdp.Value(values)
	if err != nil {
		return nil, err
	}
	script := fmt.Sprintf(
		"(() => { try { const pattern = new RegExp(%s); return {matches: %s.map(value => "+
			"pattern.test(value))}; } catch(error) { return {error: error.message}; } })()",
		p,
		v,
	)
	object, exception, err := runtime.Evaluate(script).WithReturnByValue(true).Do(protocol.WithExecutor(ctx, page))
	if err != nil {
		return nil, err
	}
	if err = evaluationException(exception); err != nil {
		return nil, err
	}
	var result struct {
		Matches *[]bool `json:"matches"`
		Error   *string `json:"error"`
	}
	if err = json.Unmarshal(object.Value, &result); err != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: err.Error(), Cause: err}
	}
	if result.Matches != nil {
		return *result.Matches, nil
	}
	if result.Error != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: *result.Error}
	}
	return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "invalid pattern result"}
}

// locatorCount counts mutually exclusive base locators.
func locatorCount(target browserproto.BrowserTarget) int {
	count := 0
	for _, present := range []bool{
		target.Ref != nil,
		target.Role != nil,
		target.Label != nil,
		target.Placeholder != nil,
		target.TextMatch != nil,
		target.TextPattern != nil,
		target.TestID != nil,
		target.CSS != nil,
	} {
		if present {
			count++
		}
	}
	return count
}

// validateTarget refuses silently overridden or modifier-only locators.
func validateTarget(target browserproto.BrowserTarget) error {
	if locatorCount(target) != 1 || ((target.Name != nil || target.NamePattern != nil) && target.Role == nil) ||
		(target.Name != nil && target.NamePattern != nil) ||
		(target.Exact != nil && *target.Exact && (target.NamePattern != nil || target.TextPattern != nil)) {
		return &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "provide exactly one of ref, role/name, label, placeholder, text-match, test-id or css",
		}
	}
	return nil
}

// hasTargetFlags prevents coordinates and URL waits from ignoring element flags.
func hasTargetFlags(target *browserproto.BrowserTarget) bool {
	return target != nil &&
		(locatorCount(
			*target,
		) > 0 || target.Name != nil || target.NamePattern != nil || target.Frame != nil || target.Exact != nil ||
			target.Nth != nil || target.Within != nil)
}

// canResample distinguishes fresh locators from retained document references.
func canResample(target browserproto.BrowserTarget) bool {
	return target.Ref == nil && target.Within == nil && target.Frame == nil
}

// resolve matches locators against the same captured observation used for output.
func (o *observation) resolve(
	ctx context.Context,
	page cdp.FrameTarget,
	target browserproto.BrowserTarget,
	refs *tabs.References,
) ([]targetElement, error) {
	if err := validateTarget(target); err != nil {
		return nil, err
	}
	scope, err := o.targetScope(target.Within, target.Frame, refs)
	if err != nil {
		return nil, err
	}
	var matches []targetElement
	if target.Ref != nil {
		matches, err = o.resolveReference(ctx, target, refs)
	} else if target.CSS != nil || target.Placeholder != nil || target.TestID != nil || target.Label != nil ||
		target.TextMatch != nil || target.TextPattern != nil {
		matches, err = o.resolveLocator(ctx, page, target)
	} else {
		matches, err = o.resolveAX(ctx, page, target)
	}
	if err != nil {
		return nil, err
	}
	if scope != nil {
		matches = slices.DeleteFunc(
			matches,
			func(element targetElement) bool {
				return !o.descendant(element.identity(), *scope, true)
			},
		)
	}
	if target.Nth != nil {
		if *target.Nth >= uint(len(matches)) {
			return []targetElement{}, nil
		}
		matches = matches[*target.Nth : *target.Nth+1]
	}
	return matches, nil
}

// resolveWait lets removal satisfy hidden/detached only in the original document.
func (o *observation) resolveWait(
	ctx context.Context,
	page cdp.FrameTarget,
	target browserproto.BrowserTarget,
	refs *tabs.References,
) ([]targetElement, error) {
	if target.Ref != nil {
		ref, ok := refs.Lookup(*target.Ref)
		if !ok || o.loaders[ref.Frame] != ref.Loader || o.pages[ref.Frame] == nil {
			return nil, &cdp.BrowserError{Kind: cdp.KindStaleReference}
		}
		if i, ok := o.domIndex[domIdentity{o.pages[ref.Frame].TargetID(), ref.Backend}]; !ok ||
			o.dom[i].frame != ref.Frame {
			return []targetElement{}, nil
		}
	}
	return o.resolve(ctx, page, target, refs)
}

// domOrder deduplicates composed-tree matches in their captured document order.
func (o *observation) domOrder(elements []targetElement) []targetElement {
	slices.SortStableFunc(
		elements,
		func(a, b targetElement) int {
			return o.domIndex[a.identity()] - o.domIndex[b.identity()]
		},
	)
	return slices.CompactFunc(elements, func(a, b targetElement) bool {
		return a.identity() == b.identity()
	})
}

func (o *observation) captureAX(
	ctx context.Context,
	frame *protocol.Frame,
	renderer cdp.FrameTarget,
	roots map[protocol.FrameID]int,
) error {
	nodes, err := accessibility.GetFullAXTree().WithFrameID(frame.ID).Do(protocol.WithExecutor(ctx, renderer))
	if err != nil {
		var chromeErr *cdp.ProtocolError
		if !errors.As(err, &chromeErr) {
			return err
		}
		o.inaccessible[frame.ID] = chromeErr.Error()
		nodes = nil
	}
	first := len(o.nodes)
	indices := map[accessibility.NodeID]int{}
	for i, node := range nodes {
		indices[node.NodeID] = first + i
	}
	if len(nodes) > 0 {
		roots[frame.ID] = first
	}
	for _, node := range nodes {
		var children []int
		for _, id := range node.ChildIDs {
			if i, ok := indices[id]; ok {
				children = append(children, i)
			}
		}
		if node.BackendDOMNodeID != 0 {
			key := domIdentity{renderer.TargetID(), node.BackendDOMNodeID}
			if _, ok := o.axIndex[key]; !ok {
				o.axIndex[key] = len(o.nodes)
			}
		}
		o.nodes = append(o.nodes, observedNode{node, frame.ID, frame.LoaderID, children})
	}
	return nil
}

func (o *observation) orderAX(roots map[protocol.FrameID]int, main protocol.FrameID) {
	var pending []nodeDepth
	if root, ok := roots[main]; ok {
		pending = append(pending, nodeDepth{root, 0})
	}
	for len(pending) > 0 {
		item := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		node := o.nodes[item.index]
		visible := !node.ax.Ignored && axText(node.ax.Role) != "InlineTextBox"
		depth := item.depth
		if visible {
			o.order = append(o.order, item)
			depth++
		}
		for i := len(node.children) - 1; i >= 0; i-- {
			pending = append(pending, nodeDepth{node.children[i], depth})
		}
	}
}

func (o *observation) captureDOM(page cdp.FrameTarget, snapshot cdp.FrameSnapshot) error {
	main, ok := snapshot.Documents[page.TargetID()]
	if !ok {
		return &cdp.BrowserError{Kind: cdp.KindStaleReference}
	}
	delete(snapshot.Documents, page.TargetID())
	type domPending struct {
		node   *protocol.Node
		frame  protocol.FrameID
		parent *domIdentity
	}
	stack := []domPending{{main.Root, snapshot.Main, nil}}
	for len(stack) > 0 {
		item := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		renderer, ok := o.pages[item.frame]
		if !ok {
			return &cdp.BrowserError{Kind: cdp.KindStaleReference}
		}
		node := item.node
		key := domIdentity{renderer.TargetID(), node.BackendNodeID}
		if item.parent != nil {
			o.parents[key] = *item.parent
		}
		if node.ContentDocument != nil {
			if node.FrameID == "" {
				return &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "frame document has no frame ID"}
			}
			stack = append(stack, domPending{node.ContentDocument, node.FrameID, &key})
		}
		if child := o.pages[node.FrameID]; child != nil {
			if document, ok := snapshot.Documents[child.TargetID()]; ok {
				delete(snapshot.Documents, child.TargetID())
				stack = append(stack, domPending{document.Root, document.FrameID, &key})
			}
		}
		children := append(slices.Clone(node.Children), node.ShadowRoots...)
		for i := len(children) - 1; i >= 0; i-- {
			stack = append(stack, domPending{children[i], item.frame, &key})
		}
		if node.NodeType == 9 {
			o.documents = append(o.documents, item.frame)
		}
		o.domIndex[key] = len(o.dom)
		o.dom = append(o.dom, observedDOM{node, item.frame})
	}
	return nil
}

func describeAXValue(
	node observedNode,
	protected bool,
	result browserproto.BrowserNode,
) (browserproto.BrowserNode, error) {
	if protected {
		result.States = append(result.States, "protected")
	} else if node.ax.Value != nil && len(node.ax.Value.Value) > 0 {
		var value any
		if err := json.Unmarshal(node.ax.Value.Value, &value); err != nil {
			return result, &cdp.BrowserError{
				Kind:    cdp.KindInvalidResult,
				Message: "AX value: " + err.Error(),
				Cause:   err,
			}
		}
		var scalar browserproto.NodeValue
		switch value := value.(type) {
		case string:
			text := browserproto.NodeValueText(value)
			scalar = &text
		case float64:
			number := browserproto.NodeValueNumber(value)
			scalar = &number
		}
		if scalar != nil {
			result.Value = &scalar
		}
	}
	return result, nil
}

func (o *observation) frameScope(
	ref browserproto.NodeRef,
	refs *tabs.References,
	expected protocol.FrameID,
) (domIdentity, protocol.FrameID, error) {
	key, err := o.reference(ref, refs)
	if err != nil {
		return domIdentity{}, "", err
	}
	item := o.dom[o.domIndex[key]]
	if item.node.LocalName != "iframe" && item.node.LocalName != "frame" {
		return domIdentity{}, "", &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "--frame must reference an iframe or frame element",
		}
	}
	if expected != "" && expected != item.frame {
		return domIdentity{}, "", &cdp.BrowserError{
			Kind:    cdp.KindConfiguration,
			Message: "--frame references must enter frames from outermost to innermost",
		}
	}
	child := item.node.FrameID
	if child == "" {
		return domIdentity{}, "", &cdp.BrowserError{
			Kind:    cdp.KindUnsupportedCapability,
			Message: "frame element has no captured child frame ID",
		}
	}
	reason, inaccessible := o.inaccessible[child]
	_, exists := o.loaders[child]
	if !exists || inaccessible {
		if !inaccessible {
			reason = "frame absent from captured tree"
		}
		return domIdentity{}, "", &cdp.BrowserError{
			Kind:    cdp.KindUnsupportedCapability,
			Message: "frame document is inaccessible: " + reason,
		}
	}

	return key, child, nil
}

func (o *observation) scanNodes(renderer cdp.FrameTarget, object *runtime.RemoteObject) ([]domIdentity, error) {
	var nodes []struct {
		Type  string `json:"type"`
		Value struct {
			Backend *protocol.BackendNodeID `json:"backendNodeId"`
		} `json:"value"`
	}
	if err := json.Unmarshal(object.DeepSerializedValue.Value, &nodes); err != nil {
		return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "locator nodes: " + err.Error(), Cause: err}
	}
	result := []domIdentity{}
	for _, node := range nodes {
		if node.Type != "node" || node.Value.Backend == nil {
			return nil, &cdp.BrowserError{
				Kind:    cdp.KindInvalidResult,
				Message: "locator nodes: expected serialized node",
			}
		}
		key := domIdentity{renderer.TargetID(), *node.Value.Backend}
		if _, ok := o.domIndex[key]; !ok {
			return nil, &cdp.BrowserError{Kind: cdp.KindStaleReference}
		}
		result = append(result, key)
	}
	return result, nil
}

func (o *observation) resolveLocator(
	ctx context.Context,
	page cdp.FrameTarget,
	target browserproto.BrowserTarget,
) ([]targetElement, error) {
	matches := []targetElement{}
	kind, expected := "text", target.TextMatch
	switch {
	case target.CSS != nil:
		kind, expected = "css", target.CSS
	case target.Label != nil:
		kind, expected = "label", target.Label
	case target.Placeholder != nil:
		kind, expected = "placeholder", target.Placeholder
	case target.TestID != nil:
		kind, expected = "test-id", target.TestID
	case target.TextPattern != nil:
		kind, expected = "text-pattern", target.TextPattern
	}
	if kind == "text-pattern" {
		if _, err := patternMatches(ctx, page, *expected, []string{}); err != nil {
			return nil, err
		}
	}
	if kind == "css" {
		if err := validateCSS(ctx, page, expected); err != nil {
			return nil, err
		}
	}
	var keys []domIdentity
	for _, frame := range o.documents {
		local, err := o.scanFrame(ctx, frame, scriptLocator, kind, *expected, target.Exact != nil && *target.Exact)
		if err != nil {
			return nil, err
		}
		keys = append(keys, local...)
	}
	slices.SortStableFunc(keys, func(a, b domIdentity) int {
		return o.domIndex[a] - o.domIndex[b]
	})
	for _, key := range keys {
		element, err := o.resolveIdentity(ctx, key)
		if err != nil {
			return nil, err
		}
		matches = append(matches, element)
	}
	return matches, nil
}

func (o *observation) resolveAX(
	ctx context.Context,
	page cdp.FrameTarget,
	target browserproto.BrowserTarget,
) ([]targetElement, error) {
	matches := []targetElement{}
	var err error
	var names []bool
	if target.NamePattern != nil {
		values := make([]string, len(o.nodes))
		for i, node := range o.nodes {
			values[i] = axText(node.ax.Name)
		}
		names, err = patternMatches(ctx, page, *target.NamePattern, values)
		if err != nil {
			return nil, err
		}
		if len(names) != len(o.nodes) {
			return nil, &cdp.BrowserError{Kind: cdp.KindInvalidResult, Message: "invalid pattern result"}
		}
	}
	for _, item := range o.dom {
		if item.node.NodeType != 1 {
			continue
		}
		key := domIdentity{o.pages[item.frame].TargetID(), item.node.BackendNodeID}
		i, ok := o.axIndex[key]
		if !ok {
			continue
		}
		node := o.nodes[i]
		if node.ax.Ignored || names != nil && !names[i] {
			continue
		}
		if target.Role != nil &&
			(len(axText(node.ax.Role)) != len(*target.Role) || !strings.EqualFold(axText(node.ax.Role), *target.Role)) {
			continue
		}
		if !matchesAXName(target, node) {
			continue
		}
		element, err := o.resolveIdentity(ctx, key)
		if err != nil {
			return nil, err
		}
		matches = append(matches, element)
	}
	return matches, nil
}

func validateCSS(ctx context.Context, page cdp.FrameTarget, expected *string) error {
	encoded, err := cdp.Value(*expected)
	if err != nil {
		return err
	}
	object, exception, err := runtime.Evaluate(fmt.Sprintf(
		"(() => { try { document.querySelector(%s); return ''; } catch (error) { return "+
			"error.message; } })()", encoded)).
		WithReturnByValue(true).
		Do(protocol.WithExecutor(ctx, page))
	if err != nil {
		return err
	}
	if err = evaluationException(exception); err != nil {
		return err
	}
	var message string
	if err = json.Unmarshal(object.Value, &message); err != nil {
		return err
	}
	if message != "" {
		return &cdp.BrowserError{Kind: cdp.KindConfiguration, Message: message}
	}
	return nil
}

func matchesAXName(target browserproto.BrowserTarget, node observedNode) bool {
	if target.Name != nil {
		name := axText(node.ax.Name)
		if target.Exact != nil && *target.Exact {
			if name != *target.Name {
				return false
			}
		} else if !strings.Contains(name, *target.Name) {
			return false
		}
	}

	return true
}

func (o *observation) resolveReference(
	ctx context.Context,
	target browserproto.BrowserTarget,
	refs *tabs.References,
) ([]targetElement, error) {
	matches := []targetElement{}
	key, err := o.reference(*target.Ref, refs)
	if err != nil {
		return nil, err
	}
	element, err := o.resolveIdentity(ctx, key)
	if err != nil {
		return nil, err
	}
	matches = append(matches, element)

	return matches, nil
}
