package main

import (
	"context"
	"fmt"
	"go/parser"
	"go/token"
	"path/filepath"
	"sort"
	"strings"

	"golang.org/x/tools/go/packages"
)

// generate loads contracts in dependency order, hiding only the generated files
// of the current batch. An overlay makes regenerated dependencies available to
// later batches even during checks, without changing files on disk.
func generate(ctx context.Context, patterns []string, ts bool, tsDir string, verify bool) error {
	pkgs, err := packages.Load(&packages.Config{Context: ctx, Mode: packages.NeedName | packages.NeedFiles | packages.NeedImports | packages.NeedDeps}, patterns...)
	if err != nil {
		return err
	}
	selected := map[string]bool{}
	for _, p := range pkgs {
		for _, problem := range p.Errors {
			return fmt.Errorf("%s", problem)
		}
		for _, filename := range p.GoFiles {
			if filepath.Base(filename) == "contract_gen.go" {
				continue
			}
			file, err := parser.ParseFile(token.NewFileSet(), filename, nil, parser.ParseComments|parser.SkipObjectResolution)
			if err != nil {
				return err
			}
			for _, comment := range file.Comments {
				if strings.Contains(comment.Text(), "+demi:") {
					selected[p.ID] = true
				}
			}
		}
	}
	levels := map[string]int{}
	var batches [][]*packages.Package
	packages.Visit(pkgs, nil, func(p *packages.Package) {
		level := 0
		for _, dependency := range p.Imports {
			level = max(level, levels[dependency.ID])
		}
		if selected[p.ID] {
			for len(batches) <= level {
				batches = append(batches, nil)
			}
			batches[level] = append(batches[level], p)
			level++
		}
		levels[p.ID] = level
	})
	// Collect output separately: ParseFile can read the overlay concurrently.
	overlay := map[string][]byte{}
	for _, batch := range batches {
		var targets []string
		stripped := map[string]bool{}
		for _, p := range batch {
			if p.PkgPath == "command-line-arguments" {
				targets = append(targets, p.GoFiles...)
			} else {
				targets = append(targets, p.PkgPath)
			}
			stripped[filepath.Join(filepath.Dir(p.GoFiles[0]), "contract_gen.go")] = true
		}
		output := map[string][]byte{}
		err := generateBatch(ctx, targets, false, "", false, stripped, overlay, func(path string, code []byte) error {
			output[path] = code
			return nil
		})
		if err != nil {
			return err
		}
		for path, code := range output {
			overlay[path] = code
		}
	}
	if ts {
		return generateBatch(ctx, patterns, true, tsDir, verify, nil, overlay, nil)
	}
	paths := make([]string, 0, len(overlay))
	for path := range overlay {
		paths = append(paths, path)
	}
	sort.Strings(paths)
	for _, path := range paths {
		if err := writeGenerated(path, overlay[path], verify); err != nil {
			return err
		}
	}
	return nil
}
