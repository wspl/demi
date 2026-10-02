//go:build darwin || linux

// Command accept runs transitional Rust and web suites against replacement programs.
//
// Run from the repository root on macOS or Linux:
//
//	CGO_ENABLED=0 GOFLAGS=-mod=readonly go run ./scripts/gomig/accept \
//	  -ref /path/to/target/debug -programs /path/to/go/programs \
//	  -suite backend_remote_host -suite runner -- test_filter --include-ignored
//
// Omit -programs (or pass an empty directory) for the Rust baseline. The web
// suite requires installed Bun dependencies and already generated contracts;
// it deliberately skips the contracts build. Test arguments after -- apply to
// every selected suite. The complete runner output precedes each summary.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"
)

type suites []string

func (s *suites) String() string { return strings.Join(*s, ",") }
func (s *suites) Set(value string) error {
	if !regexp.MustCompile(`^[A-Za-z0-9_]+$`).MatchString(value) {
		return fmt.Errorf("invalid suite %q", value)
	}
	*s = append(*s, value)
	return nil
}

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, terminationSignal())
	defer stop()
	if err := run(ctx, os.Args[1:], os.Stdout); err != nil {
		fmt.Fprintln(os.Stderr, "accept:", err)
		os.Exit(1)
	}
}

func run(ctx context.Context, args []string, output io.Writer) (err error) {
	flags := flag.NewFlagSet("accept", flag.ContinueOnError)
	flags.SetOutput(output)
	ref := flags.String("ref", "", "Rust target/debug directory (required)")
	programs := flags.String("programs", "", "directory of replacement executable programs; empty uses Rust only")
	var selected suites
	flags.Var(&selected, "suite", "suite name; repeat for multiple suites, or web")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *ref == "" || len(selected) == 0 {
		return errors.New("-ref and at least one -suite are required; put test arguments after --")
	}
	directory, err := os.MkdirTemp("", "demi-accept-")
	if err != nil {
		return fmt.Errorf("create acceptance directory: %w", err)
	}
	defer func() { err = errors.Join(err, os.RemoveAll(directory)) }()
	if err := os.WriteFile(filepath.Join(directory, ".cargo-lock"), nil, 0600); err != nil {
		return err
	}
	if err := linkPrograms(*ref, directory, false); err != nil {
		return err
	}
	// Suites also start Cargo examples, such as the web suite's scripted
	// machine manager, by their path below the profile directory.
	examples, err := filepath.Abs(filepath.Join(*ref, "examples"))
	if err != nil {
		return err
	}
	if err := os.Symlink(examples, filepath.Join(directory, "examples")); err != nil {
		return fmt.Errorf("link examples: %w", err)
	}
	if *programs != "" {
		if err := linkPrograms(*programs, directory, true); err != nil {
			return err
		}
	}
	for _, suite := range selected {
		if ctx.Err() != nil {
			return errors.Join(err, ctx.Err())
		}
		suiteErr := runSuite(ctx, *ref, directory, suite, flags.Args(), output)
		if suiteErr != nil {
			// The error is also returned; a failed progress write changes nothing.
			_, _ = fmt.Fprintf(output, "%s: %v\n", suite, suiteErr)
			err = errors.Join(err, fmt.Errorf("%s: %w", suite, suiteErr))
		}
	}
	return err
}

// linkPrograms builds the program set without wrappers that would change argv[0].
func linkPrograms(source, destination string, replace bool) error {
	source, err := filepath.Abs(source)
	if err != nil {
		return err
	}
	entries, err := os.ReadDir(source)
	if err != nil {
		return fmt.Errorf("read programs: %w", err)
	}
	count := 0
	for _, entry := range entries {
		path := filepath.Join(source, entry.Name())
		info, err := os.Stat(path)
		if err != nil {
			return err
		}
		if !info.Mode().IsRegular() || info.Mode().Perm()&0111 == 0 {
			continue
		}
		file, err := os.Open(path)
		if err != nil {
			return err
		}
		var magic [4]byte
		_, readErr := io.ReadFull(file, magic[:])
		closeErr := file.Close()
		if readErr != nil {
			return fmt.Errorf("read executable %s: %w", path, readErr)
		}
		if closeErr != nil {
			return closeErr
		}
		// ELF, Mach-O (both byte orders and fat binaries), or PE.
		native := bytes.Equal(magic[:], []byte{0x7f, 'E', 'L', 'F'}) ||
			bytes.Equal(magic[:], []byte{0xcf, 0xfa, 0xed, 0xfe}) || bytes.Equal(magic[:], []byte{0xce, 0xfa, 0xed, 0xfe}) ||
			bytes.Equal(magic[:], []byte{0xfe, 0xed, 0xfa, 0xcf}) || bytes.Equal(magic[:], []byte{0xfe, 0xed, 0xfa, 0xce}) ||
			bytes.Equal(magic[:], []byte{0xca, 0xfe, 0xba, 0xbe}) || bytes.Equal(magic[:], []byte{0xca, 0xfe, 0xba, 0xbf}) || string(magic[:2]) == "MZ"
		if !native {
			return fmt.Errorf("%s is not a native executable (script wrappers change argv[0])", path)
		}
		target := filepath.Join(destination, entry.Name())
		if replace {
			if _, err := os.Lstat(target); err != nil {
				return fmt.Errorf("replacement %s has no reference program: %w", entry.Name(), err)
			}
			if err := os.Remove(target); err != nil {
				return err
			}
		}
		if err := os.Symlink(path, target); err != nil {
			return err
		}
		count++
	}
	if count == 0 && !replace {
		return errors.New("reference directory contains no executable programs")
	}
	return nil
}

// findSuite selects the newest Cargo test executable, excluding object and metadata files.
func findSuite(ref, suite string) (string, error) {
	paths, err := filepath.Glob(filepath.Join(ref, "build", "*", "*", "out", suite+"-*"))
	if err != nil {
		return "", err
	}
	pattern := regexp.MustCompile("^" + regexp.QuoteMeta(suite) + `-[0-9a-f]+$`)
	var newest string
	var modified time.Time
	for _, path := range paths {
		if !pattern.MatchString(filepath.Base(path)) {
			continue
		}
		info, err := os.Stat(path)
		if err != nil {
			return "", err
		}
		if info.Mode().IsRegular() && info.Mode().Perm()&0111 != 0 && (newest == "" || info.ModTime().After(modified)) {
			newest, modified = path, info.ModTime()
		}
	}
	if newest == "" {
		return "", fmt.Errorf("no executable for suite %q in %s/build", suite, ref)
	}
	return newest, nil
}

// copySuite relocates the actual test executable so current_exe sees our cargo lock.
func copySuite(source, destination string) (err error) {
	input, err := os.Open(source)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, input.Close()) }()
	output, err := os.OpenFile(destination, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0700)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, output.Close()) }()
	_, err = io.Copy(output, input)
	return err
}

// webArgs reads the authoritative test command instead of keeping a second suite list.
func webArgs() ([]string, error) {
	data, err := os.ReadFile("package.json")
	if err != nil {
		return nil, err
	}
	var manifest struct {
		Scripts map[string]string `json:"scripts"`
	}
	if err := json.Unmarshal(data, &manifest); err != nil {
		return nil, err
	}
	const prefix = "bun run contracts && DEMI_TEST_PROGRAMS=target/debug bun scripts/test.ts "
	script := manifest.Scripts["test"]
	if !strings.HasPrefix(script, prefix) || strings.ContainsAny(strings.TrimPrefix(script, prefix), "\"'`$;&|<>\\\n") {
		return nil, errors.New("package.json test script changed: expected contracts step followed by plain bun scripts/test.ts arguments")
	}
	return append([]string{"scripts/test.ts"}, strings.Fields(strings.TrimPrefix(script, prefix))...), nil
}

func runSuite(ctx context.Context, ref, directory, suite string, args []string, output io.Writer) error {
	var command *exec.Cmd
	if suite == "web" {
		defaults, err := webArgs()
		if err != nil {
			return err
		}
		command = exec.CommandContext(ctx, "bun", append(defaults, args...)...)
		command.Env = append(os.Environ(), "DEMI_TEST_PROGRAMS="+directory, "NO_COLOR=1")
	} else {
		source, err := findSuite(ref, suite)
		if err != nil {
			return err
		}
		// Separate directories also allow the same suite to be requested twice.
		suiteDir, err := os.MkdirTemp(directory, suite+"-")
		if err != nil {
			return err
		}
		target := filepath.Join(suiteDir, filepath.Base(source))
		if err := copySuite(source, target); err != nil {
			return err
		}
		command = exec.CommandContext(ctx, target, args...)
	}
	var captured bytes.Buffer
	command.Stdout = io.MultiWriter(output, &captured)
	command.Stderr = command.Stdout
	command.WaitDelay = 10 * time.Second
	prepareProcess(command)
	// Progress lines are for the reader; the suite's result does not depend on them.
	_, _ = fmt.Fprintf(output, "\n=== %s ===\n", suite)
	runErr := command.Run()
	cleanupErr := cleanupProcess(command)
	summary, summaryErr := summarize(suite, captured.String())
	_, _ = fmt.Fprintln(output, summary)
	return errors.Join(runErr, cleanupErr, summaryErr)
}

var rustSummary = regexp.MustCompile(`test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;`)
var rustFailure = regexp.MustCompile(`(?m)^test (.+) \.\.\. FAILED$`)
var rustFailureList = regexp.MustCompile(`(?m)^    ([A-Za-z_][A-Za-z0-9_:]*)$`)
var bunCount = regexp.MustCompile(`(?m)^\s*(\d+) (pass|fail|skip|todo)\s*$`)
var bunFailure = regexp.MustCompile(`(?m)^\(fail\) (.+)$`)

// summarize reports runner totals and names while refusing an unrecognized result.
func summarize(suite, output string) (string, error) {
	passed, failed, ignored := 0, 0, 0
	found := false
	failures := rustFailure.FindAllStringSubmatch(output, -1)
	if index := strings.LastIndex(output, "failures:\n"); index >= 0 {
		failures = append(failures, rustFailureList.FindAllStringSubmatch(output[index:], -1)...)
	}
	if suite == "web" {
		failures = bunFailure.FindAllStringSubmatch(output, -1)
		for _, match := range bunCount.FindAllStringSubmatch(output, -1) {
			n, err := strconv.Atoi(match[1])
			if err != nil {
				return "", err
			}
			found = true
			switch match[2] {
			case "pass":
				passed += n
			case "fail":
				failed += n
			default:
				ignored += n
			}
		}
	} else {
		for _, match := range rustSummary.FindAllStringSubmatch(output, -1) {
			counts := []*int{&passed, &failed, &ignored}
			for i, count := range counts {
				n, err := strconv.Atoi(match[i+1])
				if err != nil {
					return "", err
				}
				*count += n
			}
			found = true
		}
	}
	summary := fmt.Sprintf("%s: %d passed, %d failed, %d ignored", suite, passed, failed, ignored)
	seen := make(map[string]bool)
	for _, failure := range failures {
		if !seen[failure[1]] {
			summary += "\n  FAIL " + failure[1]
			seen[failure[1]] = true
		}
	}
	if !found {
		return summary + " (counts unavailable)", errors.New("test runner produced no recognized summary")
	}
	if failed != 0 {
		return summary, fmt.Errorf("%d tests failed", failed)
	}
	return summary, nil
}
