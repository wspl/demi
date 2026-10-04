// Command archcheck enforces the Go package graph in the architecture contract.
package main

import (
	"bufio"
	"context"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"path"
	"path/filepath"
	"slices"
	"strings"

	"golang.org/x/tools/go/packages"
)

var errMissingGraph = errors.New("missing ### Go packages text block")

// programTest builds or finds the repository's programs for any test.
const programTest = "internal/programtest"

// contractRuntime is the runtime of generated contract code.
const contractRuntime = "internal/contract"

type graph map[string]map[string]bool

func main() {
	document := flag.String("graph", "docs/architecture/packages.md", "architecture document")
	flag.Parse()
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	if err := run(ctx, ".", *document); err != nil {
		fmt.Fprintln(os.Stderr, "archcheck:", err)
		stop()
		os.Exit(1)
	}
	fmt.Println("archcheck: PASS")
}

// readGraph reads Demi's authoritative package-edge block without a second table.
func readGraph(document []byte) (graph, error) {
	result := graph{}
	phase := "before"
	scanner := bufio.NewScanner(strings.NewReader(string(document)))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if phase == "before" {
			if line == "### Go packages" {
				phase = "section"
			}
			continue
		}
		if phase == "section" {
			if strings.HasPrefix(line, "#") {
				break
			}
			if line == "```text" {
				phase = "block"
			}
			continue
		}
		if line == "```" {
			phase = "closed"
			break
		}
		if line == "" {
			continue
		}
		if err := addGraphLine(result, line); err != nil {
			return nil, err
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("read graph: %w", err)
	}
	if phase == "before" || phase == "section" {
		return nil, errMissingGraph
	}
	if phase != "closed" || len(result) == 0 {
		return nil, errors.New("empty or unclosed Go package graph")
	}
	if err := checkGraphTargets(result); err != nil {
		return nil, err
	}
	return result, nil
}

// isTestdata reports whether a package lies in a testdata directory.
func isTestdata(name string) bool {
	return name == "testdata" || strings.HasPrefix(name, "testdata/") || strings.Contains(name, "/testdata/") ||
		strings.HasSuffix(name, "/testdata")
}

// supportOwner returns the package a test-support package supports: the
// support package of a/b is a/b/btest.
func supportOwner(name string) (string, bool) {
	owner := path.Dir(name)
	if owner == "." || path.Base(name) != path.Base(owner)+"test" {
		return "", false
	}
	return owner, true
}

// validPath checks the relative package names in Demi's graph.
func validPath(name string) bool {
	return name != "" && name != "." && name != "none" && !strings.HasPrefix(name, "/") &&
		!strings.HasPrefix(name, "../") && path.Clean(name) == name && !strings.ContainsAny(name, " \t\\,:>")
}

// run checks every shipping target so platform-only Demi packages cannot escape.
func run(ctx context.Context, dir, document string) error {
	data, err := os.ReadFile(document)
	if err != nil {
		return fmt.Errorf("read architecture document: %w", err)
	}
	rules, err := readGraph(data)
	if err != nil {
		return err
	}
	seen := map[string]bool{}
	for _, goos := range []string{"darwin", "linux", "windows"} {
		for _, goarch := range []string{"amd64", "arm64"} {
			if err := checkTarget(ctx, dir, rules, seen, goos, goarch); err != nil {
				return fmt.Errorf("%s/%s: %w", goos, goarch, err)
			}
		}
	}
	for name := range rules {
		if !seen[name] {
			return fmt.Errorf("graph package does not exist: %s", name)
		}
	}
	return nil
}

// checkTarget checks Demi imports, including both forms of test package.
func checkTarget(ctx context.Context, dir string, rules graph, seen map[string]bool, goos, goarch string) error {
	loaded, err := packages.Load(&packages.Config{
		Context: ctx,
		Dir:     dir,
		Tests:   true,
		Env: append(
			os.Environ(),
			"GOOS="+goos,
			"GOARCH="+goarch,
			"CGO_ENABLED=0",
			"GOWORK=off",
		),
		Mode: packages.NeedName | packages.NeedFiles | packages.NeedImports | packages.NeedDeps |
			packages.NeedModule | packages.NeedForTest,
	}, "./...")
	if err != nil {
		return fmt.Errorf("load packages: %w", err)
	}
	var problems []string
	packages.Visit(loaded, func(pkg *packages.Package) bool {
		for _, loadErr := range pkg.Errors {
			problems = append(problems, loadErr.Error())
		}
		if pkg.Module == nil || !pkg.Module.Main {
			return true
		}
		// The generated test executable lives in the build cache, not a source package.
		if pkg.Name == "main" && strings.HasSuffix(pkg.PkgPath, ".test") {
			return true
		}
		checkPackage(pkg, rules, seen, &problems)
		return true
	}, nil)
	if len(problems) != 0 {
		slices.Sort(problems)
		return errors.New(strings.Join(slices.Compact(problems), "\n"))
	}
	return nil
}

func addGraphLine(result graph, line string) error {
	source, targets, ok := strings.Cut(line, "->")
	source = strings.TrimSpace(source)
	targets = strings.TrimSpace(targets)
	if !ok || !validPath(source) || targets == "" {
		return fmt.Errorf("invalid graph line %q", line)
	}
	if _, exists := result[source]; exists {
		return fmt.Errorf("duplicate graph package %s", source)
	}
	result[source] = map[string]bool{}
	if targets == "none" {
		return nil
	}
	for _, target := range strings.Split(targets, ",") {
		target = strings.TrimSpace(target)
		if !validPath(target) || result[source][target] {
			return fmt.Errorf("invalid dependency in %q", line)
		}
		result[source][target] = true
	}
	return nil
}

func checkPackage(pkg *packages.Package, rules graph, seen map[string]bool, problems *[]string) {
	relative, err := filepath.Rel(pkg.Module.Dir, pkg.Dir)
	if err != nil {
		*problems = append(*problems, err.Error())
		return
	}
	name := filepath.ToSlash(relative)
	// Fixture packages under testdata belong to the tests that load them,
	// not to the architecture.
	if isTestdata(name) {
		return
	}
	seen[name] = true
	allowed, exists := rules[name]
	if !exists {
		*problems = append(*problems, "unlisted package: "+name)
		return
	}
	for _, imported := range pkg.Imports {
		target, ok := allowedImport(pkg, imported, name, allowed)
		if !ok {
			*problems = append(*problems, "forbidden import: "+name+" -> "+target)
		}
	}
}

func allowedImport(pkg, imported *packages.Package, name string, allowed map[string]bool) (string, bool) {
	prefix := pkg.Module.Path + "/"
	if imported.PkgPath != pkg.Module.Path && !strings.HasPrefix(imported.PkgPath, prefix) {
		return "", true
	}
	target := strings.TrimPrefix(imported.PkgPath, prefix)
	if imported.PkgPath == pkg.Module.Path {
		target = "."
	}
	if isTestdata(target) && pkg.ForTest != "" {
		return "", true
	}
	// External tests may import their own package without an architectural edge.
	if target == name && pkg.ForTest == imported.PkgPath {
		return "", true
	}
	// A test may import the support package of its own package or of
	// a listed dependency (packages.md § Go packages).
	if owner, ok := supportOwner(target); ok && pkg.ForTest != "" && (owner == name || allowed[owner]) {
		return "", true
	}
	// Any test may get the repository's programs from programtest
	// (testing.md); production code may not.
	if target == programTest && pkg.ForTest != "" {
		return "", true
	}
	// Generated contract code imports its runtime from any package
	// (packages.md § Go packages).
	if target == contractRuntime {
		return "", true
	}
	if !allowed[target] {
		return target, false
	}
	return target, true
}

func checkGraphTargets(result graph) error {
	for source, targets := range result {
		for target := range targets {
			if _, ok := result[target]; !ok {
				return fmt.Errorf("%s names unlisted dependency %s", source, target)
			}
		}
	}

	return nil
}
