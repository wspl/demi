package shell

import (
	"context"
	"encoding/json/v2"
	"fmt"
	"maps"
	"slices"
	"strings"

	"github.com/wspl/demi/go/commandtree"
)

// Declared pairs a tree with its handlers. Leaf struct literals and generated
// schemas take the place of Rust's declaration builders.
type Declared struct {
	Tree     commandtree.Node
	Handlers []CommandHandler
	Err      error
}
type CommandHandler struct {
	Path    []string
	Handler RPCHandler
}

func DeclareLeaf(leaf commandtree.Leaf, handler RPCHandler) Declared {
	d := Declared{Tree: leaf}
	if handler != nil {
		d.Handlers = []CommandHandler{{[]string{leaf.Name}, handler}}
	}
	return d
}
func DeclareGroup(name, summary string, children ...Declared) Declared {
	group := commandtree.Group{Name: name, Summary: summary}
	d := Declared{}
	for _, child := range children {
		group.Subcommands = append(group.Subcommands, child.Tree)
		for _, h := range child.Handlers {
			d.Handlers = append(d.Handlers, CommandHandler{append([]string{name}, h.Path...), h.Handler})
		}
		if d.Err == nil {
			d.Err = child.Err
		}
	}
	d.Tree = group
	return d
}

// Describe changes an input field's help without mutating the original schema.
func Describe(d Declared, field, description string) Declared {
	if d.Err != nil {
		return d
	}
	leaf, ok := d.Tree.(commandtree.Leaf)
	if !ok || leaf.Input == nil {
		d.Err = fmt.Errorf("describes %s before its input", field)
		return d
	}
	data, err := json.Marshal(leaf.Input.Document())
	if err != nil {
		d.Err = err
		return d
	}
	var document commandtree.Object
	if err := json.Unmarshal(data, &document); err != nil {
		d.Err = err
		return d
	}
	value, _ := document.Get("properties")
	properties, _ := value.(commandtree.Object)
	value, _ = properties.Get(field)
	property, ok := value.(commandtree.Object)
	if !ok {
		d.Err = fmt.Errorf("describes %s, which its input does not have", field)
		return d
	}
	property.Set("description", description)
	properties.Set(field, property)
	document.Set("properties", properties)
	leaf.Input, d.Err = commandtree.NewSchema(document)
	d.Tree = leaf
	return d
}

// CommandSet belongs to its shard. Registration and grafting are atomic.
type CommandSet struct {
	roots    []commandtree.Node
	handlers map[string]RPCHandler
}

func (s *CommandSet) Register(d Declared) error {
	name := commandtree.Name(d.Tree)
	if IsReserved(name) {
		return fmt.Errorf("command \"%s\" is reserved for shell and system commands", name)
	}
	for _, root := range s.roots {
		if commandtree.Name(root) == name {
			return fmt.Errorf("command \"%s\" is already registered", name)
		}
	}
	if err := checkDeclared(d); err != nil {
		return err
	}
	if s.handlers == nil {
		s.handlers = map[string]RPCHandler{}
	}
	for _, h := range d.Handlers {
		s.handlers[strings.Join(h.Path, "\x00")] = h.Handler
	}
	s.roots = append(s.roots, d.Tree)
	return nil
}
func checkDeclared(d Declared) error {
	name := commandtree.Name(d.Tree)
	if d.Err != nil {
		return fmt.Errorf("\"%s\": %w", name, d.Err)
	}
	if err := commandtree.Validate(d.Tree); err != nil {
		return fmt.Errorf("\"%s\": %w", name, err)
	}
	var paths [][]string
	var checkErr error
	walkCommandLeaves(d.Tree, nil, func(path []string, leaf commandtree.Leaf) {
		if checkErr != nil {
			return
		}
		if leaf.Input != nil {
			if err := commandtree.CheckInputSubset(leaf.Input); err != nil {
				checkErr = fmt.Errorf("\"%s\" %w", strings.Join(path, " "), err)
				return
			}
		}
		if leaf.Kind == commandtree.KindRPC {
			paths = append(paths, slices.Clone(path))
		}
	})
	if checkErr != nil {
		return checkErr
	}
	for _, path := range paths {
		if !slices.ContainsFunc(d.Handlers, func(h CommandHandler) bool { return slices.Equal(h.Path, path) && h.Handler != nil }) {
			return fmt.Errorf("rpc command \"%s\" has no handler", strings.Join(path, " "))
		}
	}
	for _, h := range d.Handlers {
		if !slices.ContainsFunc(paths, func(path []string) bool { return slices.Equal(path, h.Path) }) {
			return fmt.Errorf("\"%s\" is not an rpc command, so it takes no handler", strings.Join(h.Path, " "))
		}
	}
	return nil
}

// walkCommandLeaves visits each leaf with its complete declaration path.
func walkCommandLeaves(node commandtree.Node, path []string, visit func([]string, commandtree.Leaf)) {
	path = append(slices.Clone(path), commandtree.Name(node))
	switch node := node.(type) {
	case commandtree.Leaf:
		visit(path, node)
	case commandtree.Group:
		for _, child := range node.Subcommands {
			walkCommandLeaves(child, path, visit)
		}
	}
}
func (s *CommandSet) Graft(parent []string, d Declared) error {
	if len(parent) == 0 {
		return fmt.Errorf("a graft names its parent group")
	}
	index := slices.IndexFunc(s.roots, func(root commandtree.Node) bool { return commandtree.Name(root) == parent[0] })
	if index < 0 {
		return fmt.Errorf("command \"%s\" is not registered", parent[0])
	}
	root, ok := graftCommand(s.roots[index], parent[1:], d.Tree)
	if !ok {
		return fmt.Errorf("\"%s\" is not a group", strings.Join(parent, " "))
	}
	replaced := strings.Join(append(slices.Clone(parent), commandtree.Name(d.Tree)), "\x00")
	handlers := maps.Clone(s.handlers)
	var declared []CommandHandler
	for path, h := range handlers {
		parts := strings.Split(path, "\x00")
		if parts[0] != parent[0] {
			continue
		}
		if path == replaced || strings.HasPrefix(path, replaced+"\x00") {
			delete(handlers, path)
			continue
		}
		declared = append(declared, CommandHandler{parts, h})
	}
	for _, h := range d.Handlers {
		path := append(slices.Clone(parent), h.Path...)
		declared = append(declared, CommandHandler{path, h.Handler})
		handlers[strings.Join(path, "\x00")] = h.Handler
	}
	if err := checkDeclared(Declared{root, declared, d.Err}); err != nil {
		return err
	}
	s.roots[index] = root
	s.handlers = handlers
	return nil
}

// graftCommand copies the group path before replacing the named child.
func graftCommand(node commandtree.Node, path []string, child commandtree.Node) (commandtree.Node, bool) {
	group, ok := node.(commandtree.Group)
	if !ok {
		return nil, false
	}
	group.Subcommands = slices.Clone(group.Subcommands)
	if len(path) == 0 {
		index := slices.IndexFunc(group.Subcommands, func(n commandtree.Node) bool { return commandtree.Name(n) == commandtree.Name(child) })
		if index < 0 {
			group.Subcommands = append(group.Subcommands, child)
		} else {
			group.Subcommands[index] = child
		}
		return group, true
	}
	for i, n := range group.Subcommands {
		if commandtree.Name(n) == path[0] {
			replacement, ok := graftCommand(n, path[1:], child)
			if !ok {
				return nil, false
			}
			group.Subcommands[i] = replacement
			return group, true
		}
	}
	return nil, false
}
func (s *CommandSet) Filter(keep func([]string) bool) *CommandSet {
	result := &CommandSet{handlers: map[string]RPCHandler{}}
	var filter func(commandtree.Node, []string) commandtree.Node
	filter = func(node commandtree.Node, path []string) commandtree.Node {
		path = append(slices.Clone(path), commandtree.Name(node))
		switch node := node.(type) {
		case commandtree.Leaf:
			if keep(path) {
				return node
			}
		case commandtree.Group:
			children := []commandtree.Node{}
			for _, child := range node.Subcommands {
				if kept := filter(child, path); kept != nil {
					children = append(children, kept)
				}
			}
			if len(children) > 0 {
				node.Subcommands = children
				return node
			}
		}
		return nil
	}
	for _, root := range s.roots {
		if kept := filter(root, nil); kept != nil {
			result.roots = append(result.roots, kept)
		}
	}
	for path, h := range s.handlers {
		if keep(strings.Split(path, "\x00")) {
			result.handlers[path] = h
		}
	}
	return result
}
func (s *CommandSet) Declarations() []commandtree.Node { return slices.Clone(s.roots) }
func (s *CommandSet) RenderHelp() string {
	if len(s.roots) == 0 {
		return ""
	}
	parts := []string{commandtree.HelpDefaults}
	for _, root := range s.roots {
		parts = append(parts, commandtree.Help(root, commandtree.Name(root)))
	}
	return strings.Join(parts, "\n\n")
}
func (s *CommandSet) Dispatch(ctx context.Context, invocation RPCInvocation, port RPCPort) (uint8, error) {
	handler := s.handlers[strings.Join(invocation.Path, "\x00")]
	var leaf *commandtree.Leaf
	for _, root := range s.roots {
		walkCommandLeaves(root, nil, func(path []string, candidate commandtree.Leaf) {
			if slices.Equal(path, invocation.Path) {
				leaf = &candidate
			}
		})
	}
	if leaf == nil || handler == nil {
		return 0, &RPCError{RPCUsage, fmt.Sprintf("\"%s\" is not an rpc command", strings.Join(invocation.Path, " "))}
	}
	var arguments commandtree.Object
	if err := json.Unmarshal(invocation.Args, &arguments); err != nil {
		return 0, &RPCError{RPCUsage, err.Error()}
	}
	if err := leaf.CheckArguments(arguments); err != nil {
		return 0, &RPCError{RPCUsage, err.Error()}
	}
	return handler.Call(ctx, invocation, port)
}

// TypedRPC uses the arguments' generated decoder. Dispatch already checked the
// schema, so a decoding error is a declaration bug rather than bad user input.
func TypedRPC[A any](decode func([]byte) (A, error), run func(context.Context, A, RPCInvocation, RPCPort) (uint8, error)) RPCHandler {
	return RPCHandlerFunc(func(ctx context.Context, invocation RPCInvocation, port RPCPort) (uint8, error) {
		args, err := decode(invocation.Args)
		if err != nil {
			return 0, &RPCError{RPCFailed, fmt.Sprintf("the arguments of \"%s\" do not decode as declared: %s", strings.Join(invocation.Path, " "), err)}
		}
		return run(ctx, args, invocation, port)
	})
}
func IsReserved(name string) bool {
	switch name {
	case ".", "bash", "break", "cd", "command", "continue", "echo", "exit", "export", "jobs", "local", "popd", "printf", "pushd", "read", "return", "set", "sh", "shift", "source", "test", "true", "false", "unset", "wait",
		"awk", "cat", "chmod", "cp", "cut", "du", "file", "find", "grep", "head", "jq", "ls", "mkdir", "mv", "nl", "rg", "rm", "sed", "sort", "stat", "tail", "tee", "touch", "tr", "tree", "uniq", "wc", "xargs", "yq",
		"bun", "cargo", "docker", "git", "go", "node", "npm", "pnpm", "python", "python3", "ruby", "rustc", "yarn":
		return true
	}
	return false
}
