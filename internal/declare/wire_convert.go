package declare

import (
	"fmt"
)

// DecodeDeclaration reads a command tree with unpinned native operations.
func DecodeDeclaration(document []byte) (Node[NativeOperation], error) {
	wire, err := decodeRawNode(document)
	if err != nil {
		return nil, err
	}
	return nodeFromWire(wire, declarationBinding)
}

// DecodeManifestNode reads a command tree with pinned native operations.
func DecodeManifestNode(document []byte) (Node[Binding], error) {
	wire, err := decodeRawNode(document)
	if err != nil {
		return nil, err
	}
	return nodeFromWire(wire, manifestBinding)
}

// declarationBinding enforces the declaration's typed binding family.
func declarationBinding(wire rawBinding) (NativeOperation, error) {
	if binding, ok := wire.(*NativeOperation); ok {
		return *binding, nil
	}
	return NativeOperation{}, declarationError("a declaration requires an unpinned native operation")
}

// manifestBinding enforces the manifest's typed binding family.
func manifestBinding(wire rawBinding) (Binding, error) {
	if binding, ok := wire.(*Binding); ok {
		return *binding, nil
	}
	return Binding{}, declarationError("a manifest node requires a pinned binding")
}

// nodeFromWire converts the decoded tree, compiling each carried schema once.
func nodeFromWire[B any](wire rawNode, bindingFromWire func(rawBinding) (B, error)) (Node[B], error) {
	switch wire := wire.(type) {
	case *rawGroup:
		group := &Group[B]{
			Name:        wire.Name,
			Summary:     wire.Summary,
			Subcommands: make([]Node[B], 0, len(wire.Subcommands)),
		}
		for _, child := range wire.Subcommands {
			node, err := nodeFromWire(child, bindingFromWire)
			if err != nil {
				return nil, err
			}
			group.Subcommands = append(group.Subcommands, node)
		}
		return group, nil
	case *rawLeaf:
		leaf, err := leafFromWire(wire, bindingFromWire)
		if err != nil {
			return nil, err
		}
		return leaf, nil
	}
	return nil, declarationError("missing command node")
}

// MarshalJSON writes the same raw group shape as Rust's serde implementation.
func (g *Group[B]) MarshalJSON() ([]byte, error) {
	wire, err := nodeToWire[B](g)
	if err != nil {
		return nil, err
	}
	return marshalNode(wire)
}

// MarshalJSON converts the runtime leaf to Rust's RawLeaf wire representation.
func (l *Leaf[B]) MarshalJSON() ([]byte, error) {
	wire, err := nodeToWire[B](l)
	if err != nil {
		return nil, err
	}
	return marshalNode(wire)
}

// marshalNode keeps the generated codec call outside generic function bodies.
func marshalNode(wire rawNode) ([]byte, error) {
	return (rawNodeJSON{Value: wire}).MarshalJSON()
}

// nodeToWire separates command runtime state from the manifest's raw data.
func nodeToWire[B any](node Node[B]) (rawNode, error) {
	switch node := node.(type) {
	case *Group[B]:
		group := &rawGroup{
			Name:        node.Name,
			Summary:     node.Summary,
			Subcommands: make([]rawNode, 0, len(node.Subcommands)),
		}
		for _, child := range node.Subcommands {
			wire, err := nodeToWire(child)
			if err != nil {
				return nil, err
			}
			group.Subcommands = append(group.Subcommands, wire)
		}
		return group, nil
	case *Leaf[B]:
		leaf := &rawLeaf{
			Name:          node.Name,
			Summary:       node.Summary,
			SuccessOutput: node.SuccessOutput,
			FailureOutput: node.FailureOutput,
			RunningHint:   node.RunningHint,
			Positionals:   node.Positionals,
			StdinField:    node.StdinField,
			RestField:     node.RestField,
		}
		if node.Input != nil {
			document := node.Input.Document()
			leaf.Input = &document
		}
		if node.Output != nil {
			leaf.Output = &rawLeafOutput{}
			if node.Output.JSON != nil {
				document := node.Output.JSON.Document()
				leaf.Output.JSON = &document
			}
		}
		switch kind := node.Kind.(type) {
		case *RPC[B]:
			leaf.Kind = "rpc"
		case *Native[B]:
			leaf.Kind = "native"
			binding, err := bindingToWire(kind.Binding)
			if err != nil {
				return nil, err
			}
			leaf.Binding = &binding
		}
		return leaf, nil
	}
	return nil, declarationError("missing command node")
}

// bindingToWire selects the raw binding shape for the runtime binding family.
func bindingToWire(binding any) (rawBinding, error) {
	switch binding := binding.(type) {
	case NativeOperation:
		return &binding, nil
	case Binding:
		return &binding, nil
	}
	return nil, fmt.Errorf("unsupported command binding type %T", binding)
}

func leafFromWire[B any](wire *rawLeaf, bindingFromWire func(rawBinding) (B, error)) (*Leaf[B], error) {
	leaf := &Leaf[B]{
		Name:          wire.Name,
		Summary:       wire.Summary,
		SuccessOutput: wire.SuccessOutput,
		FailureOutput: wire.FailureOutput,
		RunningHint:   wire.RunningHint,
		Positionals:   wire.Positionals,
		StdinField:    wire.StdinField,
		RestField:     wire.RestField,
	}
	// Rust deserializes Schema fields before TryFrom checks kind/binding.
	if wire.Input != nil {
		schema, err := NewSchema(*wire.Input)
		if err != nil {
			return nil, err
		}
		leaf.Input = schema
	}
	if wire.Output != nil {
		leaf.Output = &LeafOutput{}
		if wire.Output.JSON != nil {
			schema, err := NewSchema(*wire.Output.JSON)
			if err != nil {
				return nil, err
			}
			leaf.Output.JSON = schema
		}
	}
	var binding B
	if wire.Binding != nil {
		var err error
		binding, err = bindingFromWire(*wire.Binding)
		if err != nil {
			return nil, err
		}
	}
	switch wire.Kind {
	case "rpc":
		if wire.Binding != nil {
			return nil, declarationError("an rpc command has no binding")
		}
		leaf.Kind = &RPC[B]{}
	case "native":
		if wire.Binding == nil {
			return nil, declarationError("a native command names its binding")
		}
		leaf.Kind = &Native[B]{Binding: binding}
	}
	return leaf, nil
}
