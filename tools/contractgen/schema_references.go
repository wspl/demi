package main

import (
	"fmt"
	"go/types"
	"maps"
	"slices"

	"github.com/wspl/demi/internal/contract"
)

// subschema returns a type's schema at a use. Commands inline named
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
	// Schema primitives and string timestamps are always inline, never a named definition.
	if primitiveSchema(d) || has(d.marks, "timestamp") {
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

// definitionName reserves a $defs name before visiting the type's fields,
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

// primitiveSchema identifies contracts whose schema is an inline primitive without
// type metadata: +demi:schema-primitive types and string timestamps.
func primitiveSchema(d *definition) bool {
	return has(d.marks, "schema-primitive") || has(d.marks, "timestamp") && !integerTimestamp(d.typ)
}
