// Command bodycheck refuses a function body with statements written on one
// line, which gofmt keeps as it finds it (AGENTS.md § Layout). With -fix it
// writes each such body on lines of its own instead.
package main

import (
	"errors"
	"flag"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
)

func main() {
	fix := flag.Bool("fix", false, "Write each one-line body on lines of its own.")
	flag.Parse()
	found, err := run(".", *fix)
	if err != nil {
		fmt.Fprintln(os.Stderr, "bodycheck:", err)
		os.Exit(1)
	}
	if len(found) > 0 && !*fix {
		for _, position := range found {
			fmt.Fprintf(os.Stderr, "%s: a function body with statements is on one line\n", position)
		}
		fmt.Fprintf(
			os.Stderr,
			"bodycheck: %d one-line bodies; go run ./tools/bodycheck -fix rewrites them\n",
			len(found),
		)
		os.Exit(1)
	}
	if *fix {
		fmt.Printf("bodycheck: rewrote %d one-line bodies\n", len(found))
		return
	}
	fmt.Println("bodycheck: PASS")
}

// run checks, or with fix rewrites, the hand-written Go files under root's
// cmd, internal and tools directories, and returns the one-line bodies found.
func run(root string, fix bool) ([]string, error) {
	var found []string
	for _, top := range []string{"cmd", "internal", "tools"} {
		err := filepath.WalkDir(filepath.Join(root, top), func(path string, entry fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if entry.IsDir() {
				if entry.Name() == "testdata" {
					return filepath.SkipDir
				}
				return nil
			}
			if !strings.HasSuffix(path, ".go") || strings.HasSuffix(path, "_gen.go") {
				return nil
			}
			positions, err := checkFile(path, fix)
			found = append(found, positions...)
			return err
		})
		if err != nil && !errors.Is(err, fs.ErrNotExist) {
			return found, err
		}
	}
	return found, nil
}

// checkFile reports the file's one-line bodies and, with fix, rewrites them.
func checkFile(path string, fix bool) ([]string, error) {
	src, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, path, src, parser.ParseComments)
	if err != nil {
		return nil, err
	}
	bodies := oneLineBodies(fset, file)
	var found []string
	for _, body := range bodies {
		found = append(found, fset.Position(body.Lbrace).String())
	}
	if !fix || len(bodies) == 0 {
		return found, nil
	}
	fixed, err := splitBodies(fset, src, bodies)
	if err != nil {
		return found, fmt.Errorf("%s: %w", path, err)
	}
	return found, os.WriteFile(path, fixed, 0o644)
}

// oneLineBodies returns each function body, declared or literal, that holds
// statements and opens and closes on one line. An empty body may stay {}.
func oneLineBodies(fset *token.FileSet, file *ast.File) []*ast.BlockStmt {
	var bodies []*ast.BlockStmt
	ast.Inspect(file, func(node ast.Node) bool {
		var body *ast.BlockStmt
		switch fn := node.(type) {
		case *ast.FuncDecl:
			body = fn.Body
		case *ast.FuncLit:
			body = fn.Body
		}
		if body != nil && len(body.List) > 0 &&
			fset.Position(body.Lbrace).Line == fset.Position(body.Rbrace).Line {
			bodies = append(bodies, body)
		}
		return true
	})
	return bodies
}

// splitBodies breaks the line after each body's opening brace and before its
// closing one, then lets gofmt indent the result.
func splitBodies(fset *token.FileSet, src []byte, bodies []*ast.BlockStmt) ([]byte, error) {
	var breaks []int
	for _, body := range bodies {
		breaks = append(breaks, fset.Position(body.Lbrace).Offset+1, fset.Position(body.Rbrace).Offset)
	}
	// From the end, so each earlier offset still points where it did.
	slices.Sort(breaks)
	slices.Reverse(breaks)
	out := slices.Clone(src)
	for _, offset := range breaks {
		out = slices.Insert(out, offset, '\n')
	}
	return format.Source(out)
}
