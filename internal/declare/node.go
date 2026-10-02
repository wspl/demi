package declare

import (
	"bytes"
	"errors"
	"fmt"
	"slices"

	"github.com/santhosh-tekuri/jsonschema/v6"
)

// MaxDepth is the deepest a command tree nests.
const MaxDepth = 32

// DeclarationError describes a broken command declaration.
type DeclarationError struct{ err error }

func (e *DeclarationError) Error() string { return e.err.Error() }

func (e *DeclarationError) Unwrap() error { return e.err }

// declarationError constructs a refusal of a command's declaration rules.
func declarationError(message string) *DeclarationError {
	return &DeclarationError{err: errors.New(message)}
}

// Node is a group or leaf. B identifies a native operation before or after pinning.
// Runtime nodes convert through the generated raw wire representation.
//
//sumtype:decl
type Node[B any] interface {
	node()
	Validate() error
	Select([]string) (*Selected[B], error)
	Help(string) string
	Leaves() []*Leaf[B]
}

// Group selects one of its subcommands.
type Group[B any] struct {
	Name        string    `json:"name"`
	Summary     string    `json:"summary"`
	Subcommands []Node[B] `json:"subcommands"`
}

// Leaf declares a command's input sources, documentation, schemas, and operation.
type Leaf[B any] struct {
	Name          string      `json:"name"`
	Summary       string      `json:"summary"`
	SuccessOutput *string     `json:"successOutput,omitempty"`
	FailureOutput *string     `json:"failureOutput,omitempty"`
	RunningHint   *string     `json:"runningHint,omitempty"`
	Input         *Schema     `json:"input,omitempty"`
	Positionals   *[]string   `json:"positionals,omitempty"`
	StdinField    *string     `json:"stdinField,omitempty"`
	RestField     *string     `json:"restField,omitempty"`
	Output        *LeafOutput `json:"output,omitempty"`
	Kind          LeafKind[B] `json:"kind"`
}

// LeafOutput declares the schema enabling --json.
type LeafOutput struct {
	JSON *Schema `json:"json,omitempty"`
}

// LeafKind names how a command runs.
//
//sumtype:decl
type LeafKind[B any] interface{ leafKind() }

// RPC runs through a backend handler.
type RPC[B any] struct{}

func (*RPC[B]) leafKind() {}

// Native runs the bound command-package operation.
type Native[B any] struct {
	Binding B `json:"binding"`
}

func (*Native[B]) leafKind() {}

// The command package operation a declaration names. The manifest pins it to
// the descriptor the backend selected for the package ([`Node::pin`]).
//
// +demi:variant rawBinding
type NativeOperation struct {
	Package   string `json:"package"`
	Operation string `json:"operation"`
}

// The command package operation a command runs, and the digest of the
// package descriptor that serves it.
//
// +demi:variant rawBinding
type Binding struct {
	Package        string `json:"package"`
	Operation      string `json:"operation"`
	DescriptorHash string `json:"descriptorHash"`
}

func (*Group[B]) node() {}
func (*Leaf[B]) node()  {}

// Name returns the command name of either node variant.
func Name[B any](node Node[B]) string {
	switch node := node.(type) {
	case *Group[B]:
		return node.Name
	case *Leaf[B]:
		return node.Name
	}
	return ""
}

// Summary returns the command summary of either node variant.
func Summary[B any](node Node[B]) string {
	switch node := node.(type) {
	case *Group[B]:
		return node.Summary
	case *Leaf[B]:
		return node.Summary
	}
	return ""
}

// AsLeaf returns the leaf or nil for a group.
func AsLeaf[B any](node Node[B]) *Leaf[B] {
	switch node := node.(type) {
	case *Group[B]:
		return nil
	case *Leaf[B]:
		return node
	}
	return nil
}

// Leaves lists the leaves in depth-first order.
func (g *Group[B]) Leaves() []*Leaf[B] {
	var leaves []*Leaf[B]
	for _, child := range g.Subcommands {
		leaves = append(leaves, child.Leaves()...)
	}
	return leaves
}

// Leaves lists this leaf.
func (l *Leaf[B]) Leaves() []*Leaf[B] { return []*Leaf[B]{l} }

// Validate checks names, depth, subcommand uniqueness and input-source rules.
func (g *Group[B]) Validate() error { return validateNode[B](g, 0) }

// Validate checks this command's name and input-source rules.
func (l *Leaf[B]) Validate() error { return validateNode[B](l, 0) }

// validateNode checks declaration rules with the root at depth zero.
func validateNode[B any](node Node[B], depth int) error {
	if depth > MaxDepth {
		return declarationError(fmt.Sprintf("command tree exceeds %d levels", MaxDepth))
	}
	if !IsCommandName(Name(node)) {
		return declarationError("invalid command name: " + Name(node))
	}
	switch node := node.(type) {
	case *Group[B]:
		if len(node.Subcommands) == 0 {
			return declarationError("command group " + node.Name + " has no subcommands")
		}
		names := make(map[string]bool)
		for _, child := range node.Subcommands {
			if names[Name(child)] {
				return declarationError("duplicate command name: " + Name(child))
			}
			names[Name(child)] = true
			if err := validateNode(child, depth+1); err != nil {
				return err
			}
		}
	case *Leaf[B]:
		return node.validateInput()
	}
	return nil
}

// Binding returns the native binding, or nil for RPC.
func (l *Leaf[B]) Binding() *B {
	switch kind := l.Kind.(type) {
	case *RPC[B]:
		return nil
	case *Native[B]:
		return &kind.Binding
	}
	return nil
}

// Properties returns a copy of the input properties as schema documents.
func (l *Leaf[B]) Properties() map[string]any {
	if l.Input == nil {
		return nil
	}
	// The immutable document was already checked and compiled by NewSchema.
	value, _ := jsonschema.UnmarshalJSON(bytes.NewReader(l.Input.document))
	object, _ := value.(map[string]any)
	properties, _ := object["properties"].(map[string]any)
	return properties
}

// Required reports whether the input requires a field.
func (l *Leaf[B]) Required(field string) bool {
	if l.Input == nil {
		return false
	}
	fields, _ := l.Input.object["required"].([]any)
	for _, name := range fields {
		if name == field {
			return true
		}
	}
	return false
}

// JSONOutput returns the JSON output schema, if declared.
func (l *Leaf[B]) JSONOutput() *Schema {
	if l.Output == nil {
		return nil
	}
	return l.Output.JSON
}

// validateInput checks that each declared source maps to an appropriate field.
func (l *Leaf[B]) validateInput() error {
	if l.Input != nil && l.Input.object["type"] != "object" {
		return declarationError("command input must describe an object")
	}
	properties := l.properties()
	fields := map[string]bool{}
	sources := slices.Clone(l.positionals())
	if l.StdinField != nil {
		sources = append(sources, *l.StdinField)
	}
	if l.RestField != nil {
		sources = append(sources, *l.RestField)
	}
	for _, field := range sources {
		if fields[field] {
			return declarationError("multiple input sources for " + field)
		}
		fields[field] = true
		if _, exists := properties[field]; !exists {
			return declarationError("input source has no schema: " + field)
		}
	}
	for _, field := range l.propertyNames() {
		if !IsCommandName(field) {
			return declarationError("invalid input name: " + field)
		}
		if !fields[field] && (field == "help" || field == "json") {
			return declarationError("reserved command option: " + field)
		}
	}
	if l.StdinField != nil && schemaType(properties[*l.StdinField]) != "string" {
		return declarationError("stdin input must be a string")
	}
	if l.RestField != nil {
		schema, _ := properties[*l.RestField].(map[string]any)
		if schemaType(schema) != "array" || schemaType(schema["items"]) != "string" {
			return declarationError("rest input must be a string array")
		}
	}
	optional := false
	for _, field := range l.positionals() {
		if l.Required(field) && optional {
			return declarationError("required positional follows optional positional")
		}
		optional = optional || !l.Required(field)
	}
	return nil
}

// IsCommandName accepts an ASCII letter/digit followed by letters, digits, _ or -.
func IsCommandName(name string) bool {
	for i := 0; i < len(name); i++ {
		b := name[i]
		if b >= 'a' && b <= 'z' || b >= 'A' && b <= 'Z' || b >= '0' && b <= '9' {
			continue
		}
		if i != 0 && (b == '_' || b == '-') {
			continue
		}
		return false
	}
	return name != ""
}

// Pin copies a declaration tree, resolving each native operation to its descriptor.
func Pin(node Node[NativeOperation], descriptorHash func(NativeOperation) (string, error)) (Node[Binding], error) {
	switch node := node.(type) {
	case *Group[NativeOperation]:
		group := &Group[Binding]{Name: node.Name, Summary: node.Summary, Subcommands: make([]Node[Binding], 0, len(node.Subcommands))}
		for _, child := range node.Subcommands {
			pinned, err := Pin(child, descriptorHash)
			if err != nil {
				return nil, err
			}
			group.Subcommands = append(group.Subcommands, pinned)
		}
		return group, nil
	case *Leaf[NativeOperation]:
		var kind LeafKind[Binding]
		switch original := node.Kind.(type) {
		case *RPC[NativeOperation]:
			kind = &RPC[Binding]{}
		case *Native[NativeOperation]:
			hash, err := descriptorHash(original.Binding)
			if err != nil {
				return nil, err
			}
			kind = &Native[Binding]{Binding: Binding{Package: original.Binding.Package, Operation: original.Binding.Operation, DescriptorHash: hash}}
		}
		leaf := &Leaf[Binding]{Name: node.Name, Summary: node.Summary, SuccessOutput: clonePointer(node.SuccessOutput), FailureOutput: clonePointer(node.FailureOutput), RunningHint: clonePointer(node.RunningHint), Input: node.Input, StdinField: clonePointer(node.StdinField), RestField: clonePointer(node.RestField), Kind: kind}
		if node.Positionals != nil {
			fields := slices.Clone(*node.Positionals)
			leaf.Positionals = &fields
		}
		if node.Output != nil {
			leaf.Output = &LeafOutput{JSON: node.Output.JSON}
		}
		return leaf, nil
	}
	return nil, nil
}

// clonePointer gives a pinned declaration ownership of its optional metadata.
func clonePointer[T any](value *T) *T {
	if value == nil {
		return nil
	}
	cloned := *value
	return &cloned
}

// positionals reads the optional command input-source list.
func (l *Leaf[B]) positionals() []string {
	if l.Positionals == nil {
		return nil
	}
	return *l.Positionals
}

// properties reads the compiled input schema without exposing mutable state.
func (l *Leaf[B]) properties() map[string]any {
	if l.Input == nil {
		return nil
	}
	properties, _ := l.Input.object["properties"].(map[string]any)
	return properties
}

// propertyNames preserves declaration order in CLI help and declaration errors.
func (l *Leaf[B]) propertyNames() []string {
	if l.Input == nil {
		return nil
	}
	return l.Input.keys("properties")
}
