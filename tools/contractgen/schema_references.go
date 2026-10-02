package main

import (
	"fmt"
	"go/types"
	"maps"
	"slices"

	"github.com/wspl/demi/internal/contract"
)

// subschema follows SchemaGenerator::subschema_for. Commands inline named
// values; plugin page and stream uses retain definitions in discovery order.
func (e *schemaEmitter) subschema(t types.Type) (any, error) {
	if pointer, ok := t.(*types.Pointer); ok {
		return e.subschema(pointer.Elem())
	}
	named, ok := t.(*types.Named)
	if !ok || !e.references || isJSON(t) {
		return e.schema(t)
	}
	key := typeKey(named)
	d := e.g.defs[key]
	// Rust's timestamp implements inline_schema explicitly.
	if has(d.marks, "timestamp") {
		return e.schema(t)
	}
	if key == e.root {
		return schemaKeywords(contract.Field{Name: "$ref", Value: "#"}), nil
	}
	name := e.names[key]
	if name == "" {
		name = e.definitionName(d)
		if _, err := e.schema(t); err != nil {
			return nil, err
		}
	}
	return schemaKeywords(contract.Field{Name: "$ref", Value: "#/$defs/" + name}), nil
}

// definitionName reserves a schemars definition before visiting its fields,
// so recursive and repeated uses share a reference without changing order.
func (e *schemaEmitter) definitionName(d *definition) string {
	if name := e.names[d.key]; name != "" {
		return name
	}
	base := d.name
	if e.references {
		base = tsName(base)
	}
	name := base
	for i := 2; slices.Contains(slices.Collect(maps.Values(e.names)), name); i++ {
		name = fmt.Sprintf("%s%d", base, i)
	}
	e.names[d.key] = name
	e.definitions.set(name, nil)
	return name
}
