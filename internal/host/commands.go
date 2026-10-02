package host

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/declare"
)

// Declared pairs a command tree with the bindings of its RPC leaves.
type Declared struct {
	tree     declare.Node[declare.NativeOperation]
	handlers map[string]RPCHandler
	err      error
}

// Served binds every RPC leaf of a declaration received as data to one handler.
func Served(tree declare.Node[declare.NativeOperation], handler RPCHandler) Declared {
	d := Declared{tree: cloneNode(tree), handlers: map[string]RPCHandler{}}
	walkLeaves(d.tree, nil, func(path []string, leaf *declare.Leaf[declare.NativeOperation]) {
		if _, ok := leaf.Kind.(*declare.RPC[declare.NativeOperation]); ok {
			d.handlers[strings.Join(path, " ")] = handler
		}
	})
	return d
}

// Group declares a group from its children, using their order.
func Group(name, summary string, children ...Declared) Declared {
	d := Declared{handlers: map[string]RPCHandler{}}
	g := &declare.Group[declare.NativeOperation]{Name: name, Summary: summary}
	for _, child := range children {
		g.Subcommands = append(g.Subcommands, cloneNode(child.tree))
		for path, handler := range child.handlers {
			d.handlers[name+" "+path] = handler
		}
		if d.err == nil {
			d.err = child.err
		}
	}
	d.tree = g
	return d
}

// Leaf declares a leaf using a struct literal and an optional RPC handler.
func Leaf(leaf declare.Leaf[declare.NativeOperation], handler RPCHandler) Declared {
	d := Declared{tree: cloneNode(&leaf), handlers: map[string]RPCHandler{}}
	if handler != nil {
		d.handlers[leaf.Name] = handler
	}
	return d
}

// Describe replaces an input field's description with text known when the command is built.
// It preserves the first refusal, which registration reports.
func (d Declared) Describe(field, description string) Declared {
	d.tree = cloneNode(d.tree)
	leaf := declare.AsLeaf(d.tree)
	var err error
	if leaf == nil || leaf.Input == nil {
		err = fmt.Errorf("describes %s before its input", field)
	} else {
		document, rewriteErr := describeSchema(leaf.Input.Document(), []string{"properties", field}, description)
		if rewriteErr != nil {
			err = fmt.Errorf("describes %s, which its input does not have", field)
		} else {
			leaf.Input, err = declare.NewSchema(document)
		}
	}
	if d.err == nil {
		d.err = err
	}
	return d
}

// describeSchema replaces a command input description without reordering its properties.
// Schema owns a checked document already; the standard decoder retains its member order here.
func describeSchema(document []byte, path []string, description string) (json.RawMessage, error) {
	decoder := json.NewDecoder(bytes.NewReader(document))
	token, err := decoder.Token()
	if err != nil {
		return nil, err
	}
	if token != json.Delim('{') {
		return nil, fmt.Errorf("schema property is not an object")
	}
	fields := []contract.Field{}
	replaced := false
	for decoder.More() {
		token, err = decoder.Token()
		if err != nil {
			return nil, err
		}
		name, ok := token.(string)
		if !ok {
			return nil, fmt.Errorf("schema property has no name")
		}
		var value json.RawMessage
		if err = decoder.Decode(&value); err != nil {
			return nil, err
		}
		if len(path) > 0 && name == path[0] {
			value, err = describeSchema(value, path[1:], description)
			if err != nil {
				return nil, err
			}
			replaced = true
		} else if len(path) == 0 && name == "description" {
			value, err = contract.EncodeJSON(description)
			if err != nil {
				return nil, err
			}
			replaced = true
		}
		fields = append(fields, contract.Field{Name: name, Value: value})
	}
	if _, err = decoder.Token(); err != nil {
		return nil, err
	}
	if !replaced {
		if len(path) > 0 {
			return nil, fmt.Errorf("schema property is absent")
		}
		fields = append(fields, contract.Field{Name: "description", Value: description})
	}
	return contract.EncodeObject(fields)
}

// RegisterError describes a refused declaration.
type RegisterError struct{ err error }

func (e *RegisterError) Error() string { return e.err.Error() }
func (e *RegisterError) Unwrap() error { return e.err }

// CommandSet holds roots in registration order and their RPC handlers.
// Its owner serializes mutations with dispatch; declarations returned to callers are copies.
type CommandSet struct {
	roots    []declare.Node[declare.NativeOperation]
	handlers map[string]RPCHandler
}

// Register adds a root atomically after checking declarations and bindings.
func (s *CommandSet) Register(d Declared) error {
	name := declare.Name(d.tree)
	if IsReserved(name) {
		return &RegisterError{fmt.Errorf("command %q is reserved for shell and system commands", name)}
	}
	for _, root := range s.roots {
		if declare.Name(root) == name {
			return &RegisterError{fmt.Errorf("command %q is already registered", name)}
		}
	}
	if err := checkDeclared(d); err != nil {
		return err
	}
	if s.handlers == nil {
		s.handlers = map[string]RPCHandler{}
	}
	for path, handler := range d.handlers {
		s.handlers[path] = handler
	}
	s.roots = append(s.roots, cloneNode(d.tree))
	return nil
}

// Graft replaces or appends a named child, checking the resulting root atomically.
func (s *CommandSet) Graft(parent []string, d Declared) error {
	if len(parent) == 0 {
		return &RegisterError{fmt.Errorf("a graft names its parent group")}
	}
	index := -1
	for i, root := range s.roots {
		if declare.Name(root) == parent[0] {
			index = i
			break
		}
	}
	if index < 0 {
		return &RegisterError{fmt.Errorf("command %q is not registered", parent[0])}
	}
	root := cloneNode(s.roots[index])
	node := findNode(root, parent[1:])
	group, ok := node.(*declare.Group[declare.NativeOperation])
	if !ok {
		return &RegisterError{fmt.Errorf("%q is not a group", strings.Join(parent, " "))}
	}
	name := declare.Name(d.tree)
	childIndex := slices.IndexFunc(group.Subcommands, func(n declare.Node[declare.NativeOperation]) bool { return declare.Name(n) == name })
	if childIndex < 0 {
		group.Subcommands = append(group.Subcommands, cloneNode(d.tree))
	} else {
		group.Subcommands[childIndex] = cloneNode(d.tree)
	}
	prefix := strings.Join(parent, " ")
	replaced := prefix + " " + name
	grafted := Declared{tree: root, handlers: map[string]RPCHandler{}, err: d.err}
	for path, handler := range s.handlers {
		if underCommand(path, parent[0]) && !underCommand(path, replaced) {
			grafted.handlers[path] = handler
		}
	}
	for path, handler := range d.handlers {
		grafted.handlers[prefix+" "+path] = handler
	}
	if err := checkDeclared(grafted); err != nil {
		return err
	}
	for path := range s.handlers {
		if underCommand(path, parent[0]) {
			delete(s.handlers, path)
		}
	}
	for path, handler := range grafted.handlers {
		s.handlers[path] = handler
	}
	s.roots[index] = root
	return nil
}

// Filter keeps accepted leaves and removes groups left empty.
func (s *CommandSet) Filter(keep func([]string) bool) *CommandSet {
	out := &CommandSet{handlers: map[string]RPCHandler{}}
	for _, root := range s.roots {
		if node := keepLeaves(root, nil, keep); node != nil {
			out.roots = append(out.roots, node)
		}
	}
	for path, handler := range s.handlers {
		if keep(strings.Split(path, " ")) {
			out.handlers[path] = handler
		}
	}
	return out
}

// Declarations returns independent roots in registration order.
func (s *CommandSet) Declarations() []declare.Node[declare.NativeOperation] {
	roots := make([]declare.Node[declare.NativeOperation], 0, len(s.roots))
	for _, root := range s.roots {
		roots = append(roots, cloneNode(root))
	}
	return roots
}

// RenderHelp prints the shared defaults and each root's help, or nothing for an empty set.
func (s *CommandSet) RenderHelp() string {
	if len(s.roots) == 0 {
		return ""
	}
	roots := make([]string, 0, len(s.roots))
	for _, root := range s.roots {
		roots = append(roots, root.Help(declare.Name(root)))
	}
	return declare.HelpDefaults + "\n\n" + strings.Join(roots, "\n\n")
}

// Dispatch validates wire arguments without coercion, then invokes the handler.
func (s *CommandSet) Dispatch(ctx context.Context, invocation RPCInvocation, port RPCPort) (uint8, error) {
	handler, err := s.Check(invocation)
	if err != nil {
		return 0, err
	}
	return handler.Call(ctx, invocation, port)
}

// Check finds the RPC handler after validating the invocation's arguments.
func (s *CommandSet) Check(invocation RPCInvocation) (RPCHandler, error) {
	named := strings.Join(invocation.Path, " ")
	var leaf *declare.Leaf[declare.NativeOperation]
	if len(invocation.Path) > 0 {
		for _, root := range s.roots {
			if declare.Name(root) == invocation.Path[0] {
				leaf = declare.AsLeaf(findNode(root, invocation.Path[1:]))
				break
			}
		}
	}
	handler := s.handlers[named]
	if leaf == nil || handler == nil {
		return nil, &RPCError{Kind: Usage, Message: fmt.Sprintf("%q is not an rpc command", named)}
	}
	err := leaf.CheckArguments(invocation.Args)
	if err != nil {
		return nil, &RPCError{Kind: Usage, Message: err.Error(), Err: err}
	}
	return handler, nil
}

// checkDeclared checks a command tree and exactly its RPC bindings.
func checkDeclared(d Declared) error {
	name := declare.Name(d.tree)
	if d.tree == nil {
		return &RegisterError{fmt.Errorf("command declaration is absent")}
	}
	if d.err != nil {
		return &RegisterError{fmt.Errorf("%q: %w", name, d.err)}
	}
	if err := d.tree.Validate(); err != nil {
		return &RegisterError{fmt.Errorf("%q: %w", name, err)}
	}
	rpc := map[string]bool{}
	var paths []string
	var refused error
	walkLeaves(d.tree, nil, func(path []string, leaf *declare.Leaf[declare.NativeOperation]) {
		if refused != nil {
			return
		}
		named := strings.Join(path, " ")
		paths = append(paths, named)
		if leaf.Input != nil {
			if err := declare.CheckInputSubset(leaf.Input); err != nil {
				refused = fmt.Errorf("%q %w", named, err)
				return
			}
		}
		switch kind := leaf.Kind.(type) {
		case *declare.RPC[declare.NativeOperation]:
			if kind != nil {
				rpc[named] = true
				return
			}
		case *declare.Native[declare.NativeOperation]:
			if kind != nil {
				return
			}
		}
		refused = fmt.Errorf("%q has no command kind", named)
	})
	if refused != nil {
		return &RegisterError{refused}
	}
	for _, path := range paths {
		if rpc[path] && d.handlers[path] == nil {
			return &RegisterError{fmt.Errorf("rpc command %q has no handler", path)}
		}
	}
	for _, path := range paths {
		if d.handlers[path] != nil && !rpc[path] {
			return &RegisterError{fmt.Errorf("%q is not an rpc command, so it takes no handler", path)}
		}
	}
	return nil
}

// walkLeaves visits command leaves in declaration order with their root-relative paths.
func walkLeaves(node declare.Node[declare.NativeOperation], path []string, visit func([]string, *declare.Leaf[declare.NativeOperation])) {
	path = append(slices.Clone(path), declare.Name(node))
	switch node := node.(type) {
	case *declare.Leaf[declare.NativeOperation]:
		visit(path, node)
	case *declare.Group[declare.NativeOperation]:
		for _, child := range node.Subcommands {
			walkLeaves(child, path, visit)
		}
	}
}

// findNode resolves a command path below one node.
func findNode(node declare.Node[declare.NativeOperation], path []string) declare.Node[declare.NativeOperation] {
	for _, name := range path {
		group, ok := node.(*declare.Group[declare.NativeOperation])
		if !ok {
			return nil
		}
		node = nil
		for _, child := range group.Subcommands {
			if declare.Name(child) == name {
				node = child
				break
			}
		}
		if node == nil {
			return nil
		}
	}
	return node
}

// keepLeaves copies the command subtree accepted by a leaf predicate.
func keepLeaves(node declare.Node[declare.NativeOperation], path []string, keep func([]string) bool) declare.Node[declare.NativeOperation] {
	path = append(slices.Clone(path), declare.Name(node))
	switch n := node.(type) {
	case *declare.Leaf[declare.NativeOperation]:
		if keep(path) {
			return cloneNode(n)
		}
	case *declare.Group[declare.NativeOperation]:
		group := &declare.Group[declare.NativeOperation]{Name: n.Name, Summary: n.Summary}
		for _, child := range n.Subcommands {
			if kept := keepLeaves(child, path, keep); kept != nil {
				group.Subcommands = append(group.Subcommands, kept)
			}
		}
		if len(group.Subcommands) > 0 {
			return group
		}
	}
	return nil
}

// cloneNode gives each command set its own declaration metadata; schemas remain immutable.
func cloneNode(node declare.Node[declare.NativeOperation]) declare.Node[declare.NativeOperation] {
	switch n := node.(type) {
	case *declare.Group[declare.NativeOperation]:
		if n == nil {
			return nil
		}
		g := *n
		g.Subcommands = make([]declare.Node[declare.NativeOperation], 0, len(n.Subcommands))
		for _, child := range n.Subcommands {
			g.Subcommands = append(g.Subcommands, cloneNode(child))
		}
		return &g
	case *declare.Leaf[declare.NativeOperation]:
		if n == nil {
			return nil
		}
		l := *n
		// declare exposes no same-binding clone. Pin changes NativeOperation to Binding,
		// and encoding would reject the unfinished declarations builders must retain.
		if n.SuccessOutput != nil {
			l.SuccessOutput = new(*n.SuccessOutput)
		}
		if n.FailureOutput != nil {
			l.FailureOutput = new(*n.FailureOutput)
		}
		if n.RunningHint != nil {
			l.RunningHint = new(*n.RunningHint)
		}
		if n.StdinField != nil {
			l.StdinField = new(*n.StdinField)
		}
		if n.RestField != nil {
			l.RestField = new(*n.RestField)
		}
		if n.Positionals != nil {
			fields := slices.Clone(*n.Positionals)
			l.Positionals = &fields
		}
		if n.Output != nil {
			output := *n.Output
			l.Output = &output
		}
		switch k := n.Kind.(type) {
		case *declare.RPC[declare.NativeOperation]:
			if k != nil {
				l.Kind = &declare.RPC[declare.NativeOperation]{}
			}
		case *declare.Native[declare.NativeOperation]:
			if k != nil {
				kind := *k
				l.Kind = &kind
			}
		}
		return &l
	}
	return nil
}

// underCommand checks a full command path component boundary.
func underCommand(path, prefix string) bool {
	return path == prefix || strings.HasPrefix(path, prefix+" ")
}
