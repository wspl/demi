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
		var scenario struct{ Name, Input, Encoded string }
		if err := json.Unmarshal(scanner.Bytes(), &scenario); err != nil {
			t.Fatal(err)
		}
		t.Run(scenario.Name, func(t *testing.T) {
			for _, value := range []any{scenario.Input, json.RawMessage(scenario.Encoded)} {
				got, err := contract.EncodeJSON(value)
				if err != nil || string(got) != scenario.Encoded {
					t.Fatalf("EncodeJSON = %s, %v; want %s", got, err, scenario.Encoded)
				}
			}
			got, err := contract.EncodeObject(
				[]contract.Field{{Name: scenario.Input, Value: map[string][]string{scenario.Input: {scenario.Input}}}},
			)
			want := "{" + scenario.Encoded + ":{" + scenario.Encoded + ":[" + scenario.Encoded + "]}}"
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

// serde_json's f32 formatter has a different fixed-notation interval from f64.
// Local scalar encodes only; budget below one second, no IO or waits.
func TestSerdeJSONFloat32(t *testing.T) {
	for _, test := range []struct {
		value float32
		want  string
	}{
		{1, "1.0"},
		{0.000001, "0.000001"},
		{0.0000001, "1e-7"},
		{1e12, "1000000000000.0"},
		{1e13, "1e+13"},
	} {
		got, err := contract.EncodeJSON(test.value)
		if err != nil || string(got) != test.want {
			t.Fatalf("%v: got %s %v, want %s", test.value, got, err, test.want)
		}
	}
}
