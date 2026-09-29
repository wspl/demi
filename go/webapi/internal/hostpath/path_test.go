package hostpath

import (
	"encoding/json"
	"os"
	"testing"
)

// Oracle: typed-path 0.12.3, Utf8TypedPath::derive(input).is_absolute().
// Pure parsing, under one second; independent of the test machine's OS.
func TestRustHostPaths(t *testing.T) {
	data, err := os.ReadFile("testdata/rust.json")
	if err != nil {
		t.Fatal(err)
	}
	var rows []struct {
		Input string
		Want  bool
	}
	if err := json.Unmarshal(data, &rows); err != nil {
		t.Fatal(err)
	}
	for _, row := range rows {
		t.Run(row.Input, func(t *testing.T) {
			if got := Absolute(row.Input); got != row.Want {
				t.Fatalf("got %v, Rust %v", got, row.Want)
			}
		})
	}
}
