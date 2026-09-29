package endpoint

import (
	"encoding/json"
	"os"
	"testing"
)

// Rust url 2.5.8 oracle: Url::parse, http/https and has_host, then to_string.
// Pure parsing, under one second; no network and no clocks.
func TestRustUserInputs(t *testing.T) {
	data, err := os.ReadFile("testdata/rust.json")
	if err != nil {
		t.Fatal(err)
	}
	var rows []struct {
		Input string
		Want  *string
	}
	if err := json.Unmarshal(data, &rows); err != nil {
		t.Fatal(err)
	}
	for _, row := range rows {
		t.Run(row.Input, func(t *testing.T) {
			value, err := Parse(row.Input)
			if row.Want == nil {
				if err == nil {
					t.Fatalf("accepted %q", value)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			if value != *row.Want {
				t.Fatalf("got %q, Rust %q", value, *row.Want)
			}
		})
	}
}
