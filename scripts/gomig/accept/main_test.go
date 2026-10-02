package main

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestMain doubles as a native Cargo-suite fixture after the harness copies it.
func TestMain(m *testing.M) {
	if strings.HasPrefix(filepath.Base(os.Args[0]), "sample-") {
		executable, err := os.Executable()
		if err != nil {
			panic(err)
		}
		directory := filepath.Dir(filepath.Dir(executable))
		_, lockErr := os.Stat(filepath.Join(directory, ".cargo-lock"))
		program, linkErr := os.Readlink(filepath.Join(directory, "demi-runner"))
		if lockErr != nil || linkErr != nil || program != os.Getenv("ACCEPT_EXPECT_PROGRAM") || strings.Join(os.Args[1:], " ") != "a_filter --include-ignored" {
			fmt.Println("test fixture::relocation ... FAILED\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;")
			os.Exit(1)
		}
		fmt.Println("test fixture::relocation ... ok\ntest result: ok. 1 passed; 0 failed; 2 ignored;")
		os.Exit(0)
	}
	os.Exit(m.Run())
}

// Cost: one local native child, normally under one second; no network or sleeps.
func TestAcceptanceRelocatesAndSubstitutes(t *testing.T) {
	root := t.TempDir()
	ref := filepath.Join(root, "reference")
	replacements := filepath.Join(root, "replacement")
	old := filepath.Join(ref, "build", "crate", "old", "out")
	newest := filepath.Join(ref, "build", "crate", "new", "out")
	for _, dir := range []string{ref, replacements, old, newest} {
		if err := os.MkdirAll(dir, 0700); err != nil {
			t.Fatal(err)
		}
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range []string{filepath.Join(ref, "demi-runner"), filepath.Join(replacements, "demi-runner"), filepath.Join(newest, "sample-abc")} {
		if err := os.Symlink(executable, path); err != nil {
			t.Fatal(err)
		}
	}
	// The older executable fails immediately if newest selection is broken.
	older := filepath.Join(old, "sample-def")
	if err := os.WriteFile(older, []byte("#!/bin/sh\nexit 99\n"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.Chtimes(older, time.Unix(1, 0), time.Unix(1, 0)); err != nil {
		t.Fatal(err)
	}
	t.Setenv("ACCEPT_EXPECT_PROGRAM", filepath.Join(replacements, "demi-runner"))
	var output bytes.Buffer
	err = run(t.Context(), []string{"-ref", ref, "-programs", replacements, "-suite", "sample", "--", "a_filter", "--include-ignored"}, &output)
	if err != nil {
		t.Fatalf("run: %v\n%s", err, output.String())
	}
	if !strings.Contains(output.String(), "sample: 1 passed, 0 failed, 2 ignored") {
		t.Fatal(output.String())
	}
}

func TestRejectsScriptReplacement(t *testing.T) {
	ref, replacements := t.TempDir(), t.TempDir()
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(executable, filepath.Join(ref, "demi-runner")); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(replacements, "demi-runner"), []byte("#!/bin/sh\nexit 0\n"), 0700); err != nil {
		t.Fatal(err)
	}
	var output bytes.Buffer
	err = run(t.Context(), []string{"-ref", ref, "-programs", replacements, "-suite", "sample"}, &output)
	if err == nil || !strings.Contains(err.Error(), "not a native executable") {
		t.Fatalf("got %v", err)
	}
}

// Cost: in-memory parsing only. These are the public runner formats, not internals.
func TestSummaries(t *testing.T) {
	for _, tc := range []struct {
		name, suite, input, want string
		fails                    bool
	}{
		{"rust", "runner", "test result: ok. 14 passed; 0 failed; 2 ignored;", "runner: 14 passed, 0 failed, 2 ignored", false},
		{"rust failure", "runner", "test jobs::cancel ... FAILED\ntest result: FAILED. 13 passed; 1 failed; 0 ignored;", "FAIL jobs::cancel", true},
		{"bun", "web", " 248 pass\n 29 fail\n 2 skip\n(fail) client > reconnect\n", "web: 248 passed, 29 failed, 2 ignored\n  FAIL client > reconnect", true},
		{"crash", "runner", "aborted", "counts unavailable", true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, err := summarize(tc.suite, tc.input)
			if (err != nil) != tc.fails || !strings.Contains(got, tc.want) {
				t.Fatalf("got %q, %v", got, err)
			}
		})
	}
}

func TestWebUsesPackageTestArguments(t *testing.T) {
	t.Chdir(t.TempDir())
	const manifest = `{"scripts":{"test":"bun run contracts && DEMI_TEST_PROGRAMS=target/debug bun scripts/test.ts --conditions development --parallel ./packages/web/src"}}`
	if err := os.WriteFile("package.json", []byte(manifest), 0600); err != nil {
		t.Fatal(err)
	}
	args, err := webArgs()
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.Join(args, " "); got != "scripts/test.ts --conditions development --parallel ./packages/web/src" {
		t.Fatal(got)
	}
	if err := os.WriteFile("package.json", []byte(`{"scripts":{"test":"bun scripts/different.ts"}}`), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := webArgs(); err == nil {
		t.Fatal("changed package test command was silently accepted")
	}
}
