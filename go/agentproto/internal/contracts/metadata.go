package main

import (
	"encoding/json/v2"
	"fmt"
	"go/ast"
	"path/filepath"
	"strings"

	"github.com/wspl/demi/go/internal/wiregen"
	"github.com/wspl/demi/go/webapi/wiredecl"
)

// Browser uses native declarations and their opaque formats. Temporary browser
// metadata retains scalar bounds, naming and custom schema shapes not in the model.
func Browser(dir string) (*wiregen.Package, wiregen.BrowserOptions, error) {
	options := wiregen.BrowserOptions{Extends: map[wiregen.BrowserType]wiregen.BrowserType{}, Aliases: map[wiregen.BrowserType]*wiregen.Type{}, Names: map[wiregen.BrowserType]string{}, Docs: map[wiregen.BrowserType]string{}, ScalarSchemas: map[wiregen.BrowserType]wiregen.BrowserScalarSchema{}}
	var pkg *wiregen.Package
	var err error
	switch filepath.Base(dir) {
	case "webapi":
		pkg, err = wiredecl.Load(dir, true)
	default:
		pkg, err = wiregen.Load(dir)
	}
	if err != nil {
		return nil, options, err
	}
	parsed, err := wiredecl.Declarations(dir)
	if err != nil {
		return nil, options, err
	}
	for _, file := range parsed[pkg.Name].Files {
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
				ref := wiregen.BrowserType{Package: pkg.Name, Name: ts.Name.Name}
				options.Names[ref] = strings.NewReplacer("API", "Api", "ID", "Id", "URL", "Url", "DTO", "Dto", "CLI", "Cli", "HTTP", "Http", "USD", "Usd").Replace(ts.Name.Name)
				if group.Doc == nil {
					continue
				}
				if doc := strings.TrimSpace(group.Doc.Text()); doc != "" {
					options.Docs[ref] = doc
				}
				for _, comment := range group.Doc.List {
					text, ok := strings.CutPrefix(comment.Text, "//wiregen:browser ")
					if !ok || strings.HasPrefix(text, "transparent ") {
						continue
					}
					if base, ok := strings.CutPrefix(text, "extends "); ok {
						options.Extends[ref] = wiregen.BrowserType{Package: pkg.Name, Name: base}
						continue
					}

					text, inline := strings.CutPrefix(text, "inline ")
					var scalar wiregen.BrowserScalarSchema
					if err := json.Unmarshal([]byte(text), &scalar, json.RejectUnknownMembers(true)); err != nil {
						return nil, options, fmt.Errorf("%s: %w", ts.Name.Name, err)
					}
					scalar.Inline = inline
					if native := pkg.OpaqueScalars[ts.Name.Name]; native != nil {
						scalar.Format = native.Format
					}
					if _, err := scalar.JSONSchema(); err != nil {
						return nil, options, fmt.Errorf("%s: %w", ts.Name.Name, err)
					}
					options.ScalarSchemas[ref] = scalar
				}
			}
		}
	}
	return pkg, options, nil
}
