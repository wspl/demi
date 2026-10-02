package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// Loading a real Go graph proves that cgo-only imports cannot disappear silently
// when cgo is disabled. These local fixtures take about one second per load.
func TestPrograms(t *testing.T) {
	for _, tc := range []struct{ name, want string }{
		{"pure", ""}, {"cgo", "build constraints exclude all Go files"}, {"broken", "missing.test/dependency"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			err := run(t.Context(), filepath.Join("testdata", tc.name))
			if tc.want == "" {
				if err != nil {
					t.Fatal(err)
				}
			} else if err == nil || !strings.Contains(err.Error(), tc.want) {
				t.Fatalf("got %v, want %q", err, tc.want)
			}
		})
	}
	t.Run("unknown program", func(t *testing.T) {
		dir := t.TempDir()
		if err := os.MkdirAll(filepath.Join(dir, "cmd", "unexpected"), 0700); err != nil {
			t.Fatal(err)
		}
		err := run(t.Context(), dir)
		if err == nil || !strings.Contains(err.Error(), "no release targets") {
			t.Fatalf("got %v", err)
		}
	})
	t.Run("no programs", func(t *testing.T) {
		dir := t.TempDir()
		if err := os.Mkdir(filepath.Join(dir, "cmd"), 0700); err != nil {
			t.Fatal(err)
		}
		err := run(t.Context(), dir)
		if err == nil || !strings.Contains(err.Error(), "no programs") {
			t.Fatalf("got %v", err)
		}
	})
}

func TestGraphRefusals(t *testing.T) {
	data, err := os.ReadFile("testdata/graphs.json")
	if err != nil {
		t.Fatal(err)
	}
	var cases []struct{ Name, Graph, Want string }
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatal(err)
	}
	for _, tc := range cases {
		t.Run(tc.Name, func(t *testing.T) {
			err := checkGraph(strings.NewReader(tc.Graph))
			if tc.Want == "" {
				if err != nil {
					t.Fatal(err)
				}
			} else if err == nil || !strings.Contains(err.Error(), tc.Want) {
				t.Fatalf("got %v, want %q", err, tc.Want)
			}
		})
	}
}
