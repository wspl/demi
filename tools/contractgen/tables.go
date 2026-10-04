package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/constant"
	"go/types"
	"reflect"
	"sort"
	"strings"

	"golang.org/x/tools/go/packages"
)

type table struct {
	name     string
	position string
	fields   *types.Struct
	scalar   string
	value    any
}

// readTable resolves a table's literal data without executing its package.
func readTable(p *packages.Package, spec *ast.ValueSpec, marks map[string]string) (*table, error) {
	fail := func(err error) (*table, error) {
		return nil, fmt.Errorf("%s: %s: %w", p.Fset.Position(spec.Pos()), spec.Names[0].Name, err)
	}
	if err := checkMarks(marks); err != nil {
		return fail(err)
	}
	if len(marks) != 1 || !has(marks, "table") || len(spec.Names) != 1 || len(spec.Values) > 1 {
		return fail(fmt.Errorf("table requires one slice variable or scalar constant and no other markers"))
	}
	obj := p.TypesInfo.Defs[spec.Names[0]]
	if obj == nil {
		return fail(fmt.Errorf("cannot resolve table declaration"))
	}
	typ := obj.Type()
	if constant, ok := obj.(*types.Const); ok {
		return readTableConstant(p, spec, constant, typ, fail)
	}
	if len(spec.Values) != 1 {
		return fail(fmt.Errorf("table variable requires a literal initializer"))
	}
	slice, ok := typ.Underlying().(*types.Slice)
	if !ok {
		return fail(fmt.Errorf("table requires a slice of structs"))
	}
	fields, ok := slice.Elem().Underlying().(*types.Struct)
	scalar := ""
	if !ok {
		if _, ok := slice.Elem().Underlying().(*types.Basic); !ok {
			return fail(fmt.Errorf("table requires structs or scalars"))
		}
		var err error
		scalar, err = tableTSType(slice.Elem())
		if err != nil {
			return fail(err)
		}
	}
	if err := checkTableFields(fields); err != nil {
		return fail(err)
	}
	value, err := tableValue(p, spec.Values[0], typ)
	if err != nil {
		return fail(err)
	}
	return &table{
		name:     strings.ToUpper(strings.Join(words(spec.Names[0].Name), "_")),
		scalar:   scalar,
		position: p.Fset.Position(spec.Pos()).String(),
		fields:   fields,
		value:    value,
	}, nil
}

// tableValue admits only literal contract data and compile-time scalar constants.
func tableValue(p *packages.Package, expr ast.Expr, typ types.Type) (any, error) {
	if v := p.TypesInfo.Types[expr].Value; v != nil {
		return tableScalar(v, typ)
	}
	literal, ok := expr.(*ast.CompositeLit)
	if !ok {
		return nil, fmt.Errorf("table values must be literals or scalar constants")
	}
	switch typ := typ.Underlying().(type) {
	case *types.Slice:
		values := make([]any, 0, len(literal.Elts))
		for _, item := range literal.Elts {
			value, err := tableValue(p, item, typ.Elem())
			if err != nil {
				return nil, err
			}
			values = append(values, value)
		}
		return values, nil
	case *types.Struct:
		return tableRow(p, literal, typ)
	}
	return nil, fmt.Errorf("unsupported table value %s", typ)
}

// tableScalar converts compile-time constants to safe TypeScript table values.
func tableScalar(v constant.Value, typ types.Type) (any, error) {
	switch v.Kind() {
	case constant.String:
		return constant.StringVal(v), nil
	case constant.Bool:
		return constant.BoolVal(v), nil
	case constant.Int:
		n, ok := constant.Int64Val(v)
		if !ok || n < -9007199254740991 || n > 9007199254740991 {
			return nil, fmt.Errorf("table integer is outside JavaScript's safe range")
		}
		return n, nil
	case constant.Float:
		n, _ := constant.Float64Val(v)
		if basic, ok := typ.Underlying().(*types.Basic); ok && basic.Kind() == types.Float32 {
			n = float64(float32(n))
		}
		return n, nil
	}
	return nil, fmt.Errorf("unsupported table constant %s", typ)
}

// tableSources emits constant data and lookups from the owning Go declarations.
func (g *generator) tableSources() ([]byte, error) {
	if len(g.tables) == 0 {
		return nil, nil
	}
	sort.Slice(g.tables, func(i, j int) bool {
		return g.tables[i].name < g.tables[j].name
	})
	var out strings.Builder
	out.WriteString(tsHeader)
	seen := map[string]bool{}
	for _, table := range g.tables {
		if seen[table.name] {
			return nil, fmt.Errorf("%s: duplicate table name %s", table.position, table.name)
		}
		seen[table.name] = true
		if err := emitTable(table, &out, seen); err != nil {
			return nil, err
		}
	}
	return []byte(out.String()), nil
}

// tableTSType describes the literal data types shared with a page.
func tableTSType(t types.Type) (string, error) {
	switch t := t.Underlying().(type) {
	case *types.Basic:
		switch {
		case t.Info()&types.IsString != 0:
			return "string", nil
		case t.Info()&types.IsBoolean != 0:
			return "boolean", nil
		case t.Info()&(types.IsInteger|types.IsFloat) != 0:
			return "number", nil
		}
	case *types.Slice:
		element, err := tableTSType(t.Elem())
		return "readonly (" + element + ")[]", err
	}
	return "", fmt.Errorf("unsupported table field type %s", t)
}

// previewTable checks the fields used by the product's established lookup API.
func previewTable(st *types.Struct) bool {
	expected := map[string]string{"mediaType": "string", "extensions": "readonly (string)[]", "inPlace": "boolean"}
	for i := 0; i < st.NumFields(); i++ {
		key := reflect.StructTag(st.Tag(i)).Get("json")
		typ, err := tableTSType(st.Field(i).Type())
		if err != nil || expected[key] != typ {
			return false
		}
	}
	return true
}

const previewLookups = `
export function previewMediaType(path: string): string | null {
 const name = path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1)
 const dot = name.lastIndexOf(".")
 if (dot <= 0) return null
 const extension = name.slice(dot + 1).replace(/[A-Z]/g, (letter) => letter.toLowerCase())
 return PREVIEW_TYPES.find((entry) => entry.extensions.includes(extension))?.mediaType ?? null
}
export function showsInPlace(mediaType: string): boolean {
 return PREVIEW_TYPES.some((entry) => entry.inPlace && entry.mediaType === mediaType)
}
`

func checkTableFields(fields *types.Struct) error {
	seen := map[string]bool{}
	for i := 0; fields != nil && i < fields.NumFields(); i++ {
		field := fields.Field(i)
		key := reflect.StructTag(fields.Tag(i)).Get("json")
		if !field.Exported() || key == "" || key == "-" || strings.Contains(key, ",") || seen[key] {
			return fmt.Errorf("%s: table fields require unique, required JSON names", field.Name())
		}
		seen[key] = true
		if _, err := tableTSType(field.Type()); err != nil {
			return fmt.Errorf("%s: %w", field.Name(), err)
		}
	}

	return nil
}

func tableRow(p *packages.Package, literal *ast.CompositeLit, typ *types.Struct) (any, error) {
	values := map[string]any{}
	seen := map[string]bool{}
	for i, item := range literal.Elts {
		index := i
		if keyed, ok := item.(*ast.KeyValueExpr); ok {
			name, ok := keyed.Key.(*ast.Ident)
			if !ok {
				return nil, fmt.Errorf("table field requires a name")
			}
			index = -1
			for j := 0; j < typ.NumFields(); j++ {
				if typ.Field(j).Name() == name.Name {
					index = j
					break
				}
			}
			item = keyed.Value
		}
		if index < 0 || index >= typ.NumFields() {
			return nil, fmt.Errorf("unknown table field")
		}
		field := typ.Field(index)
		key := reflect.StructTag(typ.Tag(index)).Get("json")
		if seen[key] {
			return nil, fmt.Errorf("duplicate table field %s", field.Name())
		}
		seen[key] = true
		value, err := tableValue(p, item, field.Type())
		if err != nil {
			return nil, fmt.Errorf("%s: %w", field.Name(), err)
		}
		values[key] = value
	}
	if len(values) != typ.NumFields() {
		return nil, fmt.Errorf("table rows must declare every field")
	}
	return values, nil
}

func emitTable(table *table, out *strings.Builder, seen map[string]bool) error {
	value, err := json.MarshalIndent(table.value, "", "  ")
	if err != nil {
		return fmt.Errorf("%s: %s: %w", table.position, table.name, err)
	}
	if table.fields == nil {
		if table.scalar == "" {
			fmt.Fprintf(out, "export const %s = %s\n", table.name, value)
			return nil
		}
		fmt.Fprintf(out, "export const %s: readonly %s[] = %s\n", table.name, table.scalar, value)
		return nil
	}
	var fields []string
	for i := 0; i < table.fields.NumFields(); i++ {
		typ, err := tableTSType(table.fields.Field(i).Type())
		if err != nil {
			return fmt.Errorf("%s: %s: %w", table.position, table.name, err)
		}
		key := reflect.StructTag(table.fields.Tag(i)).Get("json")
		fields = append(fields, "readonly "+quote(key)+": "+typ)
	}
	fmt.Fprintf(out, "export const %s: readonly { %s }[] = %s\n", table.name, strings.Join(fields, "; "), value)
	if err := emitTableLookups(table, out, seen); err != nil {
		return err
	}
	if table.name == "PREVIEW_TYPES" {
		if len(fields) != 3 || !previewTable(table.fields) {
			return fmt.Errorf("%s: PREVIEW_TYPES requires mediaType, extensions and inPlace", table.position)
		}
		out.WriteString(previewLookups)
	}
	return nil
}

func readTableConstant(
	p *packages.Package,
	spec *ast.ValueSpec,
	constant *types.Const,
	typ types.Type,
	fail func(error) (*table, error),
) (*table, error) {
	if _, err := tableTSType(typ); err != nil {
		return fail(err)
	}
	value, err := tableScalar(constant.Val(), typ)
	if err != nil {
		return fail(err)
	}
	return &table{
		name:     strings.ToUpper(strings.Join(words(spec.Names[0].Name), "_")),
		position: p.Fset.Position(spec.Pos()).String(),
		value:    value,
	}, nil
}

func emitTableLookups(table *table, out *strings.Builder, seen map[string]bool) error {
	for i := 0; i < table.fields.NumFields(); i++ {
		field := table.fields.Field(i)
		key := reflect.StructTag(table.fields.Tag(i)).Get("json")
		typ := field.Type()
		comparison := "entry[" + quote(key) + "] === value"
		if slice, ok := typ.Underlying().(*types.Slice); ok {
			typ = slice.Elem()
			comparison = "entry[" + quote(key) + "].includes(value)"
		}
		scalar, ok := typ.Underlying().(*types.Basic)
		if !ok || scalar.Info()&(types.IsString|types.IsInteger|types.IsFloat|types.IsBoolean) == 0 {
			continue
		}
		tsType, err := tableTSType(typ)
		if err != nil {
			return err
		}
		name := table.name + "By" + field.Name()
		if seen[name] {
			return fmt.Errorf("%s: duplicate table export %s", table.position, name)
		}
		seen[name] = true
		fmt.Fprintf(
			out,
			"function %s(value: %s): (typeof %s)[number] | undefined { return %s.find((entry) => %s) }\n",
			name,
			tsType,
			table.name,
			table.name,
			comparison,
		)
	}

	return nil
}
