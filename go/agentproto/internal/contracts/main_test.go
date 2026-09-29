package main

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"
)

// This checks the 11 available protocol roots against their Rust output,
// including documentation, dependency order and nested object strictness.
// Pure source generation; expected cost is under one second.
func TestAvailableProtocolIdentity(t *testing.T) {
	expected, err := os.ReadFile("testdata/protocol-available.ts")
	if err != nil {
		t.Fatal(err)
	}
	output := filepath.Join(t.TempDir(), "contracts.ts")
	t.Chdir("../../..")
	if err := run("", output); err != nil {
		t.Fatal(err)
	}
	actual, err := os.ReadFile(output)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(actual, expected) {
		t.Fatal("available protocol roots differ from Rust golden")
	}
}

// Every byte of the web module, including documentation and imports, matches Rust.
// Pure generation; expected cost is a few seconds with warm Go package metadata.
func TestWebIdentity(t *testing.T) {
	expected, err := os.ReadFile("testdata/web-api.rust.ts")
	if err != nil {
		t.Fatal(err)
	}
	output := filepath.Join(t.TempDir(), "web-api.ts")
	t.Chdir("../../..")
	if err := runWeb("", output); err != nil {
		t.Fatal(err)
	}
	actual, err := os.ReadFile(output)
	if err != nil {
		t.Fatal(err)
	}

	if !bytes.Equal(actual, expected) {
		t.Fatal("web module differs from Rust")
	}
}

// Core table contents and browser lookup functions match Rust exactly. Live
// constants are verified by the comparison command once L4 declares them.
// Pure generation; expected cost is under one second.
func TestAvailableTablesIdentity(t *testing.T) {
	expected, err := os.ReadFile("testdata/tables.rust.ts")
	if err != nil {
		t.Fatal(err)
	}
	marker := []byte("// The live view's stream")
	if index := bytes.Index(expected, marker); index >= 0 {
		expected = expected[:index]
	} else {
		t.Fatal("missing live constants boundary")
	}
	output := filepath.Join(t.TempDir(), "tables.ts")
	t.Chdir("../../..")
	if err := runTables("", output); err != nil {
		t.Fatal(err)
	}
	actual, err := os.ReadFile(output)
	if err != nil {
		t.Fatal(err)
	}
	if index := bytes.Index(actual, marker); index >= 0 {
		actual = actual[:index]
	} else {
		t.Fatal("missing live constants boundary")
	}
	if !bytes.Equal(actual, expected) {
		t.Fatal("available tables differ from Rust")
	}
}
