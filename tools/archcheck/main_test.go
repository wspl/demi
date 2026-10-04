package main

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// These fixtures exercise real Go package loading, including external tests and
// platform files. The six-target success case costs several seconds; no process
// is left running, and the tests never download a module or wait for wall time.
func TestArchitecture(t *testing.T) {
	cases := []struct {
		name   string
		graph  string
		file   string
		source string
		// support adds internal/core/coretest, the test support of core.
		support bool
		want    string
	}{
		{name: "declared imports and external self import"},
		{
			name:  "production import",
			graph: "internal/core -> none\ninternal/framewire -> none",
			want:  "forbidden import",
		},
		{
			name:   "internal test import",
			file:   "internal/core/core_test.go",
			source: "package core\nimport _ \"archcheck.test/fixture/internal/independent\"",
			want:   "forbidden import",
		},
		{
			name:   "external test import",
			file:   "internal/core/core_test.go",
			source: "package core_test\nimport _ \"archcheck.test/fixture/internal/independent\"",
			want:   "forbidden import",
		},
		{name: "unlisted package", file: "internal/extra/extra.go", source: "package extra", want: "unlisted package"},
		{
			name:  "nonexistent package",
			graph: "internal/core -> none\ninternal/framewire -> internal/core\ninternal/missing -> none",
			want:  "does not exist",
		},
		{
			name:   "load failure",
			file:   "internal/core/core.go",
			source: "package core\nimport _ \"missing.test/package\"",
			want:   "missing.test/package",
		},
		{
			name:   "platform test import",
			file:   "internal/core/core_windows_arm64_test.go",
			source: "package core_test\nimport _ \"archcheck.test/fixture/internal/independent\"",
			want:   "forbidden import",
		},
		{
			name:    "test imports a dependency's support",
			graph:   supportGraph,
			support: true,
			file:    "internal/framewire/support_test.go",
			source:  "package framewire_test\nimport _ \"archcheck.test/fixture/internal/core/coretest\"",
		},
		{
			name:    "production imports support",
			graph:   supportGraph,
			support: true,
			file:    "internal/framewire/support.go",
			source:  "package framewire\nimport _ \"archcheck.test/fixture/internal/core/coretest\"",
			want:    "forbidden import: internal/framewire -> internal/core/coretest",
		},
		{
			name:    "test imports support of no dependency",
			graph:   supportGraph,
			support: true,
			file:    "internal/independent/support_test.go",
			source:  "package independent_test\nimport _ \"archcheck.test/fixture/internal/core/coretest\"",
			want:    "forbidden import: internal/independent -> internal/core/coretest",
		},
		{
			name:   "test imports a testdata fixture",
			file:   "internal/core/testdata/fixture/fixture.go",
			source: "package fixture",
		},
		{
			name:   "test imports its fixture package",
			file:   "internal/framewire/fixture_test.go",
			source: "package framewire_test\nimport _ \"archcheck.test/fixture/internal/framewire/testdata/specimen\"",
		},
		{
			name:   "production imports a testdata package",
			file:   "internal/framewire/fixture.go",
			source: "package framewire\nimport _ \"archcheck.test/fixture/internal/framewire/testdata/specimen\"",
			want:   "forbidden import: internal/framewire -> internal/framewire/testdata/specimen",
		},
		{
			name:   "test imports programtest",
			graph:  programGraph,
			file:   "internal/framewire/programs_test.go",
			source: "package framewire_test\nimport _ \"archcheck.test/fixture/internal/programtest\"",
		},
		{
			name:   "production imports programtest",
			graph:  programGraph,
			file:   "internal/framewire/programs.go",
			source: "package framewire\nimport _ \"archcheck.test/fixture/internal/programtest\"",
			want:   "forbidden import: internal/framewire -> internal/programtest",
		},
		{
			name:   "platform package exists",
			graph:  "internal/core -> none\ninternal/framewire -> internal/core\ninternal/platform -> none",
			file:   "internal/platform/platform_linux.go",
			source: "package platform",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			if err := os.CopyFS(dir, os.DirFS("testdata/module")); err != nil {
				t.Fatal(err)
			}
			data, err := os.ReadFile("testdata/graph.md")
			if err != nil {
				t.Fatal(err)
			}
			if tc.graph != "" {
				data = []byte("### Go packages\n```text\ninternal/independent -> none\n" + tc.graph + "\n```\n")
			}
			document := filepath.Join(dir, "graph.md")
			if err := os.WriteFile(document, data, 0o600); err != nil {
				t.Fatal(err)
			}
			if tc.support {
				file := filepath.Join(dir, "internal/core/coretest/coretest.go")
				if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(
					file,
					[]byte("package coretest\nimport _ \"archcheck.test/fixture/internal/core\""),
					0o600,
				); err != nil {
					t.Fatal(err)
				}
			}
			if tc.graph == programGraph {
				programs := filepath.Join(dir, "internal/programtest/programtest.go")
				if err := os.MkdirAll(filepath.Dir(programs), 0o700); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(programs, []byte("package programtest"), 0o600); err != nil {
					t.Fatal(err)
				}
			}
			specimen := filepath.Join(dir, "internal/framewire/testdata/specimen/specimen.go")
			if err := os.MkdirAll(filepath.Dir(specimen), 0o700); err != nil {
				t.Fatal(err)
			}
			if err := os.WriteFile(specimen, []byte("package specimen"), 0o600); err != nil {
				t.Fatal(err)
			}
			if tc.file != "" {
				file := filepath.Join(dir, tc.file)
				if err := os.MkdirAll(filepath.Dir(file), 0o700); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(file, []byte(tc.source), 0o600); err != nil {
					t.Fatal(err)
				}
			}
			err = run(t.Context(), dir, document)
			if tc.want == "" {
				if err != nil {
					t.Fatal(err)
				}
			} else if err == nil || !strings.Contains(err.Error(), tc.want) {
				t.Fatalf("got %v, want %q", err, tc.want)
			}
		})
	}
}

// supportGraph declares core's test support; framewire depends on core, and
// internal/independent on nothing.
// programGraph lists programtest, which no package names as a dependency.
const programGraph = "internal/core -> none\ninternal/framewire -> internal/core\ninternal/programtest -> none"

const supportGraph = "internal/core -> none\ninternal/core/coretest -> internal/core\n" +
	"internal/framewire -> internal/core"

func TestGraphRefusals(t *testing.T) {
	for _, tc := range []struct{ name, body string }{
		{"malformed edge", "internal/core none"},
		{"absolute path", "/internal/core -> none"},
		{"parent path", "../core -> none"},
		{"duplicate package", "internal/core -> none\ninternal/core -> none"},
		{"duplicate edge", "internal/core -> internal/core, internal/core"},
		{"unlisted dependency", "internal/core -> internal/missing"},
		{"empty", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, err := readGraph([]byte("### Go packages\n```text\n" + tc.body + "\n```"))
			if err == nil {
				t.Fatal("invalid graph accepted")
			}
		})
	}
	t.Run("missing", func(t *testing.T) {
		_, err := readGraph([]byte("### Rust crates\n```text\ncore -> none\n```"))
		if !errors.Is(err, errMissingGraph) {
			t.Fatalf("got %v, want missing graph", err)
		}
	})
	t.Run("unclosed", func(t *testing.T) {
		if _, err := readGraph([]byte("### Go packages\n```text\ninternal/core -> none")); err == nil {
			t.Fatal("unclosed graph accepted")
		}
	})
}
