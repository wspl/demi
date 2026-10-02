package programtest

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"
)

type build struct {
	done      chan struct{}
	path      string
	directory string
	err       error
}

var programs = struct {
	sync.Mutex // Protects the build registry, never a running build.
	builds     map[string]*build
}{builds: make(map[string]*build)}

// Path returns a supplied program or builds cmd/name once per test binary.
// TestMain must call Run to remove build directories after all tests finish.
func Path(ctx context.Context, name string) (string, error) {
	if name == "" || name == "." || name == ".." || strings.ContainsAny(name, `/\`) {
		return "", fmt.Errorf("invalid program name %q", name)
	}
	filename := name
	if runtime.GOOS == "windows" {
		filename += ".exe"
	}
	if directory := os.Getenv("DEMI_TEST_PROGRAMS"); directory != "" {
		path, err := filepath.Abs(filepath.Join(directory, filename))
		if err != nil {
			return "", err
		}
		info, err := os.Stat(path)
		if err != nil {
			return "", fmt.Errorf("supplied program %s: %w", name, err)
		}
		if info.IsDir() {
			return "", fmt.Errorf("supplied program %s is a directory", path)
		}
		return path, nil
	}
	programs.Lock()
	pending, exists := programs.builds[name]
	if !exists {
		pending = &build{done: make(chan struct{})}
		programs.builds[name] = pending
	}
	programs.Unlock()
	if !exists {
		pending.path, pending.directory, pending.err = buildProgram(ctx, name, filename)
		close(pending.done)
	}
	select {
	case <-ctx.Done():
		return "", ctx.Err()
	case <-pending.done:
		return pending.path, pending.err
	}
}

func buildProgram(ctx context.Context, name, filename string) (string, string, error) {
	root, err := os.Getwd()
	if err != nil {
		return "", "", err
	}
	for {
		if _, err := os.Stat(filepath.Join(root, "go.mod")); err == nil {
			break
		}
		parent := filepath.Dir(root)
		if parent == root {
			return "", "", errors.New("cannot find repository go.mod")
		}
		root = parent
	}
	directory, err := os.MkdirTemp("", "demi-programtest-")
	if err != nil {
		return "", "", err
	}
	path := filepath.Join(directory, filename)
	command := exec.CommandContext(ctx, "go", "build", "-o", path, "./cmd/"+name)
	command.Dir = root
	command.Env = append(os.Environ(), "CGO_ENABLED=0", "GOFLAGS=-mod=readonly")
	output, err := command.CombinedOutput()
	if err != nil {
		return "", directory, fmt.Errorf("build %s: %w\n%s", name, err, output)
	}
	return path, directory, nil
}

// Run runs a package's tests and removes every program it built afterward.
// Use os.Exit(programtest.Run(m)) in TestMain; a leak checker can wrap Run.
func Run(m *testing.M) int {
	code := m.Run()
	programs.Lock()
	builds := programs.builds
	programs.builds = make(map[string]*build)
	programs.Unlock()
	for _, built := range builds {
		<-built.done
		if built.directory != "" {
			if err := os.RemoveAll(built.directory); err != nil {
				// Diagnostics cannot recover a cleanup failure; the exit code reports it.
				_, _ = fmt.Fprintf(os.Stderr, "programtest cleanup: %v\n", err)
				code = 1
			}
		}
	}
	return code
}
