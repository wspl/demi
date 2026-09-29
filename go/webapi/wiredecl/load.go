// Package wiredecl temporarily bridges web-only declaration gaps remaining
// after a826cb18: custom array schemas and trimmed strings. Foreign embedded
// structs are the generator's own (fe342af4).
package wiredecl

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

// Load uses native declarations, foreign embedded structs included. Its
// temporary directory stays inside the module so foreign owners resolve.
func Load(dir string, browser bool) (result *wiregen.Package, failure error) {
	parsed, err := Declarations(dir)
	if err != nil {
		return nil, err
	}
	temp, err := os.MkdirTemp(dir, ".wiredecl-")
	if err != nil {
		return nil, err
	}
	defer func() { failure = errors.Join(failure, os.RemoveAll(temp)) }()
	docs := map[string]map[string]string{}
	transparent := map[string]bool{}
	for _, pkg := range parsed {
		for name, file := range pkg.Files {
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
					structure, ok := ts.Type.(*ast.StructType)
					if !ok {
						continue
					}
					marked := false
					if group.Doc != nil {
						for _, comment := range group.Doc.List {
							if browser && strings.HasPrefix(comment.Text, "//wiregen:browser transparent ") {
								transparent[ts.Name.Name] = true
							}
							if strings.HasPrefix(comment.Text, "//demi:wire") || strings.HasPrefix(comment.Text, "//demi:variant") {
								marked = true
							}
						}
						if transparent[ts.Name.Name] {
							for _, comment := range group.Doc.List {
								if comment.Text == "//demi:opaque" {
									comment.Text = "//demi:wire"
									marked = true
								}
							}
						}
					}
					if !marked {
						continue
					}
					docs[ts.Name.Name] = map[string]string{}
					for _, field := range structure.Fields.List {
						if len(field.Names) == 1 && field.Doc != nil {
							docs[ts.Name.Name][field.Names[0].Name] = strings.TrimSpace(field.Doc.Text())
						}
					}
				}
			}
			file.Comments = nil
			var source bytes.Buffer
			if err := format.Node(&source, pkg.Fset, file); err != nil {
				return nil, err
			}
			if err := os.WriteFile(filepath.Join(temp, name), source.Bytes(), 0644); err != nil {
				return nil, err
			}
		}
	}
	pkg, err := wiregen.Load(temp)
	if err != nil {
		return nil, err
	}
	for name, fields := range docs {
		if s := pkg.Structs[name]; s != nil {
			for _, field := range s.Fields {
				if doc, ok := fields[field.Name]; ok {
					field.Doc = doc
				}
			}
		}
	}
	for name := range transparent {
		s := pkg.Structs[name]
		if s == nil || len(s.Fields) != 1 {
			return nil, fmt.Errorf("%s: custom scalar must have one representation field", name)
		}
		s.Scalar = s.Fields[0].Type
		s.Scalar.Rules = s.Fields[0].Rules
		s.Fields = nil
	}
	return pkg, nil
}

// Generate delegates codecs to wiregen; only Trimmed's missing decode hook is
// substituted. Native foreign codecs, enums, patches and embeddings stay intact.
func Generate(dir string) error {
	pkg, err := Load(dir, false)
	if err != nil {
		return err
	}
	outputs, err := pkg.GenerateGo()
	if err != nil {
		return err
	}
	for name, data := range outputs {
		fset := token.NewFileSet()
		file, err := parser.ParseFile(fset, name, data, parser.ParseComments)
		if err != nil {
			return err
		}
		ast.Inspect(file, func(node ast.Node) bool {
			if call, ok := node.(*ast.CallExpr); ok {
				if id, ok := call.Fun.(*ast.Ident); ok && id.Name == "Trimmed" {
					id.Name = "NewTrimmed"
				}
			}
			return true
		})
		var out bytes.Buffer
		if err := format.Node(&out, fset, file); err != nil {
			return err
		}
		if err := os.WriteFile(filepath.Join(dir, name), out.Bytes(), 0644); err != nil {
			return err
		}
	}
	return nil
}
