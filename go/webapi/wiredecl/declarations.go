package wiredecl

import (
	"go/ast"
	"go/build"
	"go/parser"
	"go/token"
	"path/filepath"
	"strings"
)

type declarationPackage struct {
	Files map[string]*ast.File
	Fset  *token.FileSet
}

// Declarations reads the active wire declarations, excluding generated
// codecs and tests. Build selection needs no compiled foreign owner exports.
func Declarations(dir string) (map[string]declarationPackage, error) {
	selected, err := build.Default.ImportDir(dir, 0)
	if err != nil {
		return nil, err
	}
	files := map[string]*ast.File{}
	fset := token.NewFileSet()
	for _, name := range selected.GoFiles {
		if strings.HasSuffix(name, "_wire.go") {
			continue
		}
		file, err := parser.ParseFile(fset, filepath.Join(dir, name), nil, parser.ParseComments)
		if err != nil {
			return nil, err
		}
		files[name] = file
	}
	return map[string]declarationPackage{selected.Name: {Files: files, Fset: fset}}, nil
}
