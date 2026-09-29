package commandtree

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"errors"
	"fmt"
	"slices"

	"github.com/wspl/demi/go/internal/wire"
)

// MaxDepth is the deepest a command tree nests.
const MaxDepth = 32

// An InvalidError means a document from outside the process, or a node about to
// leave it, breaks the rules of its wire type. It names the field and the rule,
// never the value.
type InvalidError = wire.InvalidError

// A Node is a command group or a command. It is the manifest's node and the
// declaration's: a leaf names its native operation with a [Binding], and a
// declaration's binding has no descriptor hash until [Pin] fills it in.
//
// A Node is told apart by its members: a group has subcommands and a leaf has a
// kind. Use [Name], [Summary] and [Leaves] on any node, and a type switch to
// tell a [Group] from a [Leaf].
//
//demi:union untagged
type Node interface {
	node()
}

// A Group is a command group: a name that only selects one of its
// subcommands.
//
//demi:variant
type Group struct {
	Name        string `json:"name"`
	Summary     string `json:"summary"`
	Subcommands []Node `json:"subcommands"`
}

// A Leaf is a command: its help texts, the JSON Schema of its input object,
// where its input comes from on the command line, and how it runs.
//
//demi:variant
type Leaf struct {
	Name          string  `json:"name"`
	Summary       string  `json:"summary"`
	SuccessOutput *string `json:"successOutput,omitzero"`
	FailureOutput *string `json:"failureOutput,omitzero"`
	RunningHint   *string `json:"runningHint,omitzero"`
	// Input is the JSON Schema of the input object; nil for a command that
	// takes no input.
	Input       *Schema     `json:"input,omitzero"`
	Positionals *[]string   `json:"positionals,omitzero"`
	StdinField  *string     `json:"stdinField,omitzero"`
	RestField   *string     `json:"restField,omitzero"`
	Output      *LeafOutput `json:"output,omitzero"`
	// Kind says how the command runs, and Binding names the operation of a
	// native command: an rpc command has none and a native command has one.
	Kind    LeafKind `json:"kind" check:"oneof=rpc|native"`
	Binding *Binding `json:"binding,omitzero"`
}

func (Group) node() {}
func (Leaf) node()  {}

// A LeafKind says how a command runs: as a call to the backend, or as an
// operation of a native command package.
type LeafKind string

// The kinds of a command.
const (
	KindRPC    LeafKind = "rpc"
	KindNative LeafKind = "native"
)

// check is the rule across a leaf's kind and binding.
func (l Leaf) check() error {
	switch {
	case l.Kind == KindRPC && l.Binding != nil:
		return errors.New("an rpc command has no binding")
	case l.Kind == KindNative && l.Binding == nil:
		return errors.New("a native command names its binding")
	}
	return nil
}

// A LeafOutput holds the JSON Schema of a command's `--json` output.
//
//demi:wire
type LeafOutput struct {
	JSON *Schema `json:"json,omitzero"`
}

// A NativeOperation is the native package operation that a declaration names.
// The manifest pins it to the descriptor the backend selected for the package
// ([Pin]).
//
//demi:wire
type NativeOperation struct {
	Package   string `json:"package"`
	Operation string `json:"operation"`
}

// A Binding is the native package operation a command runs, and the digest of
// the package descriptor that serves it.
//
//demi:wire
type Binding struct {
	Package        string `json:"package"`
	Operation      string `json:"operation"`
	DescriptorHash string `json:"descriptorHash"`
}

// DecodeNode decodes one JSON document into a node: the manifest's node, or a
// declaration's. It refuses a document that is not one, naming the field, and
// compiles every schema in it. It does not check the tree's rules: that is
// [Validate].
func DecodeNode(data []byte) (Node, error) {
	return decode[Node](data)
}

// EncodeNode returns the JSON of a node, after checking it as [DecodeNode]
// would.
func EncodeNode(node Node) ([]byte, error) {
	return encode(node)
}

// Name returns the name of a node.
func Name(node Node) string {
	switch node := node.(type) {
	case Group:
		return node.Name
	case Leaf:
		return node.Name
	}
	return ""
}

// Summary returns the summary of a node.
func Summary(node Node) string {
	switch node := node.(type) {
	case Group:
		return node.Summary
	case Leaf:
		return node.Summary
	}
	return ""
}

// Leaves returns the leaves of a tree, depth first.
func Leaves(node Node) []Leaf {
	switch node := node.(type) {
	case Group:
		var leaves []Leaf
		for _, child := range node.Subcommands {
			leaves = append(leaves, Leaves(child)...)
		}
		return leaves
	case Leaf:
		return []Leaf{node}
	}
	return nil
}

// Validate checks the tree's rules: names, at most [MaxDepth] levels, groups
// with distinctly named subcommands, and each leaf's input declaration. It
// returns a [*DeclarationError].
func Validate(root Node) error {
	return validateAt(root, 0)
}

func validateAt(node Node, depth int) error {
	if depth > MaxDepth {
		return declarationf("command tree exceeds %d levels", MaxDepth)
	}
	if !IsCommandName(Name(node)) {
		return declarationf("invalid command name: %s", Name(node))
	}
	switch node := node.(type) {
	case Group:
		if len(node.Subcommands) == 0 {
			return declarationf("command group %s has no subcommands", node.Name)
		}
		names := map[string]bool{}
		for _, child := range node.Subcommands {
			if names[Name(child)] {
				return declarationf("duplicate command name: %s", Name(child))
			}
			names[Name(child)] = true
			if err := validateAt(child, depth+1); err != nil {
				return err
			}
		}
	case Leaf:
		return node.checkDeclaration()
	}
	return nil
}

// Pin returns the manifest node of a declaration: each native command pinned to
// the descriptor that descriptorHash names for its operation. It shares the
// declaration's schemas and strings with the result, which nothing modifies.
func Pin(node Node, descriptorHash func(NativeOperation) (string, error)) (Node, error) {
	switch node := node.(type) {
	case Group:
		pinned := node
		pinned.Subcommands = make([]Node, len(node.Subcommands))
		for i, child := range node.Subcommands {
			var err error
			pinned.Subcommands[i], err = Pin(child, descriptorHash)
			if err != nil {
				return nil, err
			}
		}
		return pinned, nil
	case Leaf:
		if node.Kind == KindNative {
			if node.Binding == nil {
				return nil, declarationf("a native command names its binding: %s", node.Name)
			}
			operation := NativeOperation{Package: node.Binding.Package, Operation: node.Binding.Operation}
			hash, err := descriptorHash(operation)
			if err != nil {
				return nil, err
			}
			node.Binding = &Binding{Package: operation.Package, Operation: operation.Operation, DescriptorHash: hash}
		}
		return node, nil
	}
	return nil, fmt.Errorf("commandtree: %T is not a node", node)
}

// Properties returns the properties of the input object, in the order the
// schema declares them, when the leaf declares an input.
func (l Leaf) Properties() (Object, bool) {
	if l.Input == nil {
		return Object{}, false
	}
	member, _ := l.Input.document.Get("properties")
	properties, ok := member.(Object)
	return properties, ok
}

// IsRequired reports whether the input's schema requires the field.
func (l Leaf) IsRequired(field string) bool {
	if l.Input == nil {
		return false
	}
	member, _ := l.Input.document.Get("required")
	names, _ := member.([]any)
	return slices.Contains(names, any(field))
}

// JSONOutput returns the schema of the command's `--json` output, or nil when
// it has none.
func (l Leaf) JSONOutput() *Schema {
	if l.Output == nil {
		return nil
	}
	return l.Output.JSON
}

// checkDeclaration checks a leaf's input declaration: where each field comes
// from on the command line, and that no field breaks the command line's rules.
func (l Leaf) checkDeclaration() error {
	if l.Input != nil && typeOf(l.Input.document) != "object" {
		return declarationf("command input must describe an object")
	}
	fields := map[string]bool{}
	sources := slices.Clone(valueOf(l.Positionals))
	if l.StdinField != nil {
		sources = append(sources, *l.StdinField)
	}
	if l.RestField != nil {
		sources = append(sources, *l.RestField)
	}
	properties, hasProperties := l.Properties()
	for _, field := range sources {
		if fields[field] {
			return declarationf("multiple input sources for %s", field)
		}
		fields[field] = true
		if _, ok := properties.Get(field); !hasProperties || !ok {
			return declarationf("input source has no schema: %s", field)
		}
	}
	for field := range properties.All() {
		if !IsCommandName(field) {
			return declarationf("invalid input name: %s", field)
		}
		if !fields[field] && (field == "help" || field == "json") {
			return declarationf("reserved command option: %s", field)
		}
	}
	if l.StdinField != nil && l.propertyType(*l.StdinField) != "string" {
		return declarationf("stdin input must be a string")
	}
	if l.RestField != nil {
		schema, _ := properties.Get(*l.RestField)
		items := typeOf(lookup(schema, []string{"items"}))
		if l.propertyType(*l.RestField) != "array" || items != "string" {
			return declarationf("rest input must be a string array")
		}
	}
	optional := false
	for _, field := range valueOf(l.Positionals) {
		if l.IsRequired(field) && optional {
			return declarationf("required positional follows optional positional")
		}
		optional = optional || !l.IsRequired(field)
	}
	return nil
}

// propertyType returns the JSON type a declared input property has, or "".
func (l Leaf) propertyType(field string) string {
	properties, _ := l.Properties()
	schema, _ := properties.Get(field)
	return typeOf(schema)
}

// typeOf returns the "type" of a schema when it is a string, or "".
func typeOf(schema any) string {
	object, ok := schema.(Object)
	if !ok {
		return ""
	}
	kind, _ := object.Get("type")
	name, _ := kind.(string)
	return name
}

// valueOf returns what an optional member holds, or the zero value when it is
// absent.
func valueOf[T any](optional *T) T {
	var zero T
	if optional == nil {
		return zero
	}
	return *optional
}

// IsCommandName reports whether name can name a command or an input: an ASCII
// letter or digit, then letters, digits, `_` and `-`.
func IsCommandName(name string) bool {
	if name == "" {
		return false
	}
	for i := 0; i < len(name); i++ {
		c := name[i]
		alphanumeric := c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9'
		if !alphanumeric && (i == 0 || c != '_' && c != '-') {
			return false
		}
	}
	return true
}
