package wiredecl

import (
	"fmt"
	"go/ast"
	"reflect"
	"strconv"
	"strings"
)

// foreignFields reuses the actual owner's fields for a temporary flattened view.
func foreignFields(dir, qualifier, name string) ([]*ast.Field, error) {
	parsed, err := Declarations(dir)
	if err != nil {
		return nil, err
	}
	for _, pkg := range parsed {
		for _, file := range pkg.Files {
			for _, decl := range file.Decls {
				group, ok := decl.(*ast.GenDecl)
				if !ok {
					continue
				}
				for _, spec := range group.Specs {
					ts, ok := spec.(*ast.TypeSpec)
					if !ok || ts.Name.Name != name {
						continue
					}
					structure, ok := ts.Type.(*ast.StructType)
					if !ok {
						return nil, fmt.Errorf("%s.%s is not a struct", qualifier, name)
					}
					var qualify func(ast.Expr) ast.Expr
					qualify = func(expr ast.Expr) ast.Expr {
						switch t := expr.(type) {
						case *ast.Ident:
							if ast.IsExported(t.Name) {
								return &ast.SelectorExpr{X: ast.NewIdent(qualifier), Sel: t}
							}
						case *ast.StarExpr:
							t.X = qualify(t.X)
						case *ast.ArrayType:
							t.Elt = qualify(t.Elt)
						case *ast.MapType:
							t.Value = qualify(t.Value)
						}
						return expr
					}
					for _, field := range structure.Fields.List {
						field.Type = qualify(field.Type)
						if field.Tag != nil {
							tag, err := strconv.Unquote(field.Tag.Value)
							if err != nil {
								return nil, err
							}
							rules := strings.ReplaceAll(reflect.StructTag(tag).Get("check"), "func=Validate", "func="+qualifier+".Validate")
							var foreignRule func(ast.Expr) string
							foreignRule = func(expr ast.Expr) string {
								switch t := expr.(type) {
								case *ast.SelectorExpr:
									return "func=" + qualifier + ".Validate"
								case *ast.StarExpr:
									return foreignRule(t.X)
								case *ast.ArrayType:
									if inner := foreignRule(t.Elt); inner != "" {
										return "each(" + inner + ")"
									}
								}
								return ""
							}
							if !strings.Contains(rules, "func="+qualifier+".Validate") {
								if extra := foreignRule(field.Type); extra != "" {
									if rules != "" {
										rules += ","
									}
									rules += extra
								}
							}
							tag = "json:" + strconv.Quote(reflect.StructTag(tag).Get("json"))
							if rules != "" {
								tag += " check:" + strconv.Quote(rules)
							}
							field.Tag.Value = strconv.Quote(tag)
						}
					}
					return structure.Fields.List, nil
				}
			}
		}
	}
	return nil, fmt.Errorf("required shared type %s.%s is not declared in %s", qualifier, name, dir)
}
