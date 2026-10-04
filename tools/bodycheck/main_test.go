package main

import (
	"os"
	"path/filepath"
	"testing"
)

// Cost: one temporary module tree of three files; no build or process.
func TestOneLineBodiesAreFoundAndSplit(t *testing.T) {
	root := t.TempDir()
	write := func(path, text string) {
		t.Helper()
		full := filepath.Join(root, path)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(full, []byte(text), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	write("internal/a/a.go", `package a

func Empty() {}

func Short() int { return 1 }

func Outer(close func() error) {
	defer func() { _ = close() }()
	done := func() bool { return true }
	_ = done
}
`)
	write("internal/a/a_gen.go", "package a\n\nfunc Generated() int { return 2 }\n")
	write("internal/a/testdata/fixture.go", "package fixture\n\nfunc F() int { return 3 }\n")

	found, err := run(root, false)
	if err != nil {
		t.Fatal(err)
	}
	if len(found) != 3 {
		t.Fatalf("got %d one-line bodies %v, want 3: Short and the two literals", len(found), found)
	}
	if _, err := run(root, true); err != nil {
		t.Fatal(err)
	}
	found, err = run(root, false)
	if err != nil || len(found) != 0 {
		t.Fatalf("after -fix: %v, %v; want none", found, err)
	}
	got, err := os.ReadFile(filepath.Join(root, "internal/a/a.go"))
	if err != nil {
		t.Fatal(err)
	}
	want := `package a

func Empty() {}

func Short() int {
	return 1
}

func Outer(close func() error) {
	defer func() {
		_ = close()
	}()
	done := func() bool {
		return true
	}
	_ = done
}
`
	if string(got) != want {
		t.Fatalf("fixed file:\n%s\nwant:\n%s", got, want)
	}
}
