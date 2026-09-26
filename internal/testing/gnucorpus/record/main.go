// Command record runs the GNU (or other) reference implementation for every
// utility's case definitions and writes the recorded corpus to
// testdata/gnu/<utility>/cases.json. It is run explicitly, never as part of
// `go test`:
//
//	go run ./internal/testing/gnucorpus/record [utility ...]
//
// With no arguments it records every utility. See testdata/gnu/README.md.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"syscall"
	"time"

	"github.com/wspl/demi/internal/testing/gnucorpus"
	"github.com/wspl/demi/internal/testing/gnucorpus/record/definitions"
)

// referenceTimeout bounds one case's run of the reference implementation, so
// a case that hangs (e.g. a malformed `sleep` or `tail -f`) fails the
// recording instead of hanging it.
const referenceTimeout = 10 * time.Second

func main() {
	utilities := os.Args[1:]
	if len(utilities) == 0 {
		utilities = definitions.Order
	}
	for _, name := range utilities {
		def, ok := definitions.Registry[name]
		if !ok {
			fmt.Fprintf(os.Stderr, "record: no definitions for %q\n", name)
			os.Exit(1)
		}
		cases, err := record(name, def)
		if err != nil {
			fmt.Fprintf(os.Stderr, "record: %s: %v\n", name, err)
			os.Exit(1)
		}
		if err := write(name, cases); err != nil {
			fmt.Fprintf(os.Stderr, "record: %s: write: %v\n", name, err)
			os.Exit(1)
		}
		fmt.Printf("recorded %s: %d cases\n", name, len(cases))
	}
}

// record runs def.Reference for every case definitions.Utility supplies and
// returns the corpus with Want filled in.
func record(utility string, def definitions.Utility) ([]gnucorpus.Case, error) {
	cases := def.Cases()
	for i := range cases {
		want, err := runReference(def.Reference, cases[i])
		if err != nil {
			return nil, fmt.Errorf("case %s: %w", cases[i].Name, err)
		}
		cases[i].Want = want
	}
	return cases, nil
}

// runReference runs the reference binary for one case in a fresh sandbox
// built the same way gnucorpus.Check builds one for the Go utility, so the
// two are compared on identical footing.
func runReference(binary string, c gnucorpus.Case) (gnucorpus.Want, error) {
	root, err := os.MkdirTemp("", "gnucorpus-record-*")
	if err != nil {
		return gnucorpus.Want{}, err
	}
	// Best-effort cleanup of a recording sandbox: its removal failing does
	// not affect what was recorded, and the OS reclaims the temp dir anyway.
	defer func() { _ = os.RemoveAll(root) }()

	work := filepath.Join(root, "work")
	home := filepath.Join(root, "home")
	for _, dir := range []string{work, home} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			return gnucorpus.Want{}, err
		}
	}
	if err := gnucorpus.WriteTree(work, c.Tree); err != nil {
		return gnucorpus.Want{}, fmt.Errorf("set up tree: %w", err)
	}

	previousUmask := syscall.Umask(0o022)
	defer syscall.Umask(previousUmask)

	ctx, cancel := context.WithTimeout(context.Background(), referenceTimeout)
	defer cancel()

	cmd := exec.CommandContext(ctx, binary, c.Argv...)
	cmd.Dir = work
	cmd.Env = envSlice(gnucorpus.BaseEnv(home), c.Env)
	cmd.Stdin = bytes.NewReader(c.Stdin)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	exitCode := 0
	if err := cmd.Run(); err != nil {
		var exitErr *exec.ExitError
		if !errors.As(err, &exitErr) {
			return gnucorpus.Want{}, fmt.Errorf("run %s: %w", binary, err)
		}
		exitCode = exitErr.ExitCode()
	}

	tree, err := gnucorpus.SnapshotTree(work)
	if err != nil {
		return gnucorpus.Want{}, fmt.Errorf("snapshot tree: %w", err)
	}
	tree.Times, err = gnucorpus.TimesOf(work, c.CheckTimes)
	if err != nil {
		return gnucorpus.Want{}, fmt.Errorf("read mtimes: %w", err)
	}

	return gnucorpus.Want{
		Stdout:         stdout.Bytes(),
		ExitCode:       exitCode,
		StderrNonEmpty: stderr.Len() > 0,
		Tree:           tree,
	}, nil
}

// envSlice renders a case's environment as a process environment slice, in
// a stable order so recording is reproducible byte for byte.
func envSlice(base, extra map[string]string) []string {
	merged := gnucorpus.MergeEnv(base, extra)
	names := make([]string, 0, len(merged))
	for name := range merged {
		names = append(names, name)
	}
	sort.Strings(names)
	env := make([]string, 0, len(names))
	for _, name := range names {
		env = append(env, name+"="+merged[name])
	}
	return env
}

// write serializes cases as indented JSON to testdata/gnu/<utility>/cases.json.
func write(utility string, cases []gnucorpus.Case) error {
	path, err := gnucorpus.CorpusPath(utility)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(cases, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	return os.WriteFile(path, data, 0o644)
}
