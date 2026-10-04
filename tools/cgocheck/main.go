// Command cgocheck verifies each shipping program's pure-Go dependency graph.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
)

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	if err := run(ctx, "."); err != nil {
		fmt.Fprintln(os.Stderr, "cgocheck:", err)
		stop()
		os.Exit(1)
	}
	fmt.Println("cgocheck: PASS")
}

// programTargets applies the executable release table to Demi's command directories.
func programTargets(program string) ([]string, error) {
	switch program {
	case "demi-runner", "demi-file", "demi-browser", "demi-claude-code", "demi-native-fixture":
		// The native fixture follows the runner, whose acceptance tests start it.
		return []string{"darwin", "linux", "windows"}, nil
	case "demi-backend":
		return []string{"darwin", "linux"}, nil
	case "demi-machine-manager":
		return []string{"linux"}, nil
	default:
		return nil, fmt.Errorf("no release targets declared for cmd/%s", program)
	}
}

// run inspects every program, refusing new programs without a release policy.
func run(ctx context.Context, dir string) error {
	entries, err := os.ReadDir(filepath.Join(dir, "cmd"))
	if err != nil {
		return fmt.Errorf("read command directories: %w", err)
	}
	count := 0
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		targets, err := programTargets(entry.Name())
		if err != nil {
			return err
		}
		count++
		for _, goos := range targets {
			for _, goarch := range []string{"amd64", "arm64"} {
				if err := checkProgram(ctx, dir, "./cmd/"+entry.Name(), goos, goarch); err != nil {
					return fmt.Errorf("%s %s/%s: %w", entry.Name(), goos, goarch, err)
				}
			}
		}
	}
	if count == 0 {
		return errors.New("no programs found in cmd")
	}
	return nil
}

// checkProgram asks Go for the selected no-cgo graph, including load errors.
func checkProgram(ctx context.Context, dir, program, goos, goarch string) error {
	cmd := exec.CommandContext(ctx, "go", "list", "-e", "-deps", "-json", program)
	cmd.Dir = dir
	cmd.Env = append(
		os.Environ(),
		"GOOS="+goos,
		"GOARCH="+goarch,
		"CGO_ENABLED=0",
		"GOWORK=off",
	)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	output, err := cmd.Output()
	if err != nil {
		return fmt.Errorf("load dependency graph: %w: %s", err, stderr.String())
	}
	return checkGraph(bytes.NewReader(output))
}

// checkGraph rejects cgo and incomplete graphs reported by the Go tool.
func checkGraph(input io.Reader) error {
	decoder := json.NewDecoder(input)
	count := 0
	for {
		var pkg struct {
			ImportPath string
			CgoFiles   []string
			Incomplete bool
			Error      *struct{ Err string }
			DepsErrors []struct{ Err string }
		}
		err := decoder.Decode(&pkg)
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return fmt.Errorf("decode dependency graph: %w", err)
		}
		if pkg.ImportPath == "" {
			return errors.New("dependency has no import path")
		}
		count++
		if len(pkg.CgoFiles) != 0 {
			return fmt.Errorf("cgo files in %s: %v", pkg.ImportPath, pkg.CgoFiles)
		}
		if pkg.Error != nil {
			return fmt.Errorf("load %s: %s", pkg.ImportPath, pkg.Error.Err)
		}
		if len(pkg.DepsErrors) != 0 {
			return fmt.Errorf("load dependency of %s: %s", pkg.ImportPath, pkg.DepsErrors[0].Err)
		}
		if pkg.Incomplete {
			return fmt.Errorf("incomplete dependency graph for %s", pkg.ImportPath)
		}
	}
	if count == 0 {
		return errors.New("empty dependency graph")
	}
	return nil
}
