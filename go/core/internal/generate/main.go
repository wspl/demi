// Command generate adapts core's declarations to wiregen until lane L5 adds
// embedded tagged variants and default-false fields. It does not own any wire
// shape: fields and rules always come from the embedded declaration.
package main

import (
	"bytes"
	"errors"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"strings"

	"github.com/wspl/demi/go/internal/wiregen"
)

func main() {
	if err := generate(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

// generate expands core's two unsupported declarations only in temporary input.
func generate() (result error) {
	dir, err := os.MkdirTemp("", "demi-core-wiregen-")
	if err != nil {
		return err
	}
	defer func() { result = errors.Join(result, os.RemoveAll(dir)) }()
	fset := token.NewFileSet()
	entries, err := os.ReadDir(".")
	if err != nil {
		return err
	}
	files := map[string]*ast.File{}
	structs := map[string]*ast.StructType{}
	marked := map[string]bool{}
	for _, entry := range entries {
		name := entry.Name()
		if !strings.HasSuffix(name, ".go") || strings.HasSuffix(name, "_test.go") || strings.HasSuffix(name, "_wire.go") {
			continue
		}
		file, err := parser.ParseFile(fset, name, nil, parser.ParseComments)
		if err != nil {
			return err
		}
		files[name] = file
		for _, decl := range file.Decls {
			group, ok := decl.(*ast.GenDecl)
			if !ok {
				continue
			}
			for _, spec := range group.Specs {
				ts, ok := spec.(*ast.TypeSpec)
				if !ok {
					continue
				}
				if structure, ok := ts.Type.(*ast.StructType); ok {
					structs[ts.Name.Name] = structure
					marked[ts.Name.Name] = false
					if group.Doc != nil {
						for _, comment := range group.Doc.List {
							if strings.HasPrefix(comment.Text, "//demi:wire") || strings.HasPrefix(comment.Text, "//demi:variant") {
								marked[ts.Name.Name] = true
							}
						}
					}
				}
			}
		}
	}
	defaults := map[string][]string{}
	for name, structure := range structs {
		if !marked[name] {
			continue
		}
		var fields []*ast.Field
		for _, field := range structure.Fields.List {
			if len(field.Names) == 0 {
				embedded, ok := field.Type.(*ast.Ident)
				if !ok || structs[embedded.Name] == nil {
					return fmt.Errorf("%s: unsupported embedded type", name)
				}
				fields = append(fields, structs[embedded.Name].Fields.List...)
			} else {
				fields = append(fields, field)
			}
		}
		structure.Fields.List = fields
		for _, field := range fields {
			if len(field.Names) == 1 && field.Names[0].Name == "Forkable" {
				// A temporary pointer satisfies wiregen's parser. Restore the model to
				// bool below, keeping presence optional and refusing a JSON null.
				field.Type = &ast.StarExpr{X: ast.NewIdent("bool")}
				defaults[name] = append(defaults[name], "Forkable")
			}
		}
	}
	for name, file := range files {
		var source bytes.Buffer
		if err := format.Node(&source, fset, file); err != nil {
			return err
		}
		if err := os.WriteFile(filepath.Join(dir, name), source.Bytes(), 0600); err != nil {
			return err
		}
	}
	pkg, err := wiregen.Load(dir)
	if err != nil {
		return err
	}
	for name, names := range defaults {
		for _, field := range pkg.Structs[name].Fields {
			for _, name := range names {
				if field.Name == name {
					field.Type = field.Type.Elem
				}
			}
		}
	}
	outputs, err := pkg.GenerateGo()
	if err != nil {
		return err
	}
	for name, data := range outputs {
		if err := os.WriteFile(name, data, 0644); err != nil {
			return err
		}
	}
	return nil
}
