package contract_test

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/contract"
)

// Captured serde_json bytes cover every runtime encoding position.
// In-memory cases and one small fixture read; budget <1 second.
func TestSerdeJSONEscaping(t *testing.T) {
	data, err := os.ReadFile("testdata/escaping.jsonl")
	if err != nil {
		t.Fatal(err)
	}
	scanner := bufio.NewScanner(bytes.NewReader(data))
	for scanner.Scan() {
		var tc struct{ Name, Input, Encoded string }
		if err := json.Unmarshal(scanner.Bytes(), &tc); err != nil {
			t.Fatal(err)
		}
		t.Run(tc.Name, func(t *testing.T) {
			for _, value := range []any{tc.Input, json.RawMessage(tc.Encoded)} {
				got, err := contract.EncodeJSON(value)
				if err != nil || string(got) != tc.Encoded {
					t.Fatalf("EncodeJSON = %s, %v; want %s", got, err, tc.Encoded)
				}
			}
			got, err := contract.EncodeObject([]contract.Field{{Name: tc.Input, Value: map[string][]string{tc.Input: {tc.Input}}}})
			want := "{" + tc.Encoded + ":{" + tc.Encoded + ":[" + tc.Encoded + "]}}"
			if err != nil || string(got) != want {
				t.Fatalf("EncodeObject = %s, %v; want %s", got, err, want)
			}
		})
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
}

// Rust's oracle checks the same container counts. No IO or waits; budget <1 s.
func TestSerdeJSONRecursionLimit(t *testing.T) {
	for _, shape := range []struct{ name, open, close string }{{"array", "[", "]"}, {"object", `{"x":`, "}"}} {
		for _, depth := range []int{126, 127, 128, 129} {
			for _, leaf := range []string{"0", "[]", "{}"} {
				t.Run(fmt.Sprintf("%s/%d/%s", shape.name, depth, leaf), func(t *testing.T) {
					count := depth
					if leaf != "0" {
						count++
					}
					input := strings.Repeat(shape.open, depth) + leaf + strings.Repeat(shape.close, depth)
					err := contract.CheckJSON([]byte(input))
					if (err == nil) != (count < 128) {
						t.Fatalf("%d containers: %v", count, err)
					}
				})
			}
		}
	}
}
