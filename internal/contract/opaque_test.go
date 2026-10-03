package contract_test

import (
	"bufio"
	"bytes"
	"encoding/json"
	"os"
	"testing"

	"github.com/wspl/demi/internal/contract"
)

// The golden corpus pins each number's kind and spelling independently of Go's formatter.
// The local corpus costs less than a second and starts no processes.
func TestOpaqueNumberCorpus(t *testing.T) {
	data, err := os.ReadFile("testdata/numbers.jsonl")
	if err != nil {
		t.Fatal(err)
	}
	scanner := bufio.NewScanner(bytes.NewReader(data))
	for scanner.Scan() {
		var row struct {
			Input   string `json:"input"`
			Encoded string `json:"encoded"`
		}
		if err := json.Unmarshal(scanner.Bytes(), &row); err != nil {
			t.Fatal(err)
		}
		t.Run(row.Input, func(t *testing.T) {
			data, err := contract.EncodeMsgpack(json.RawMessage(row.Input))
			if err != nil {
				t.Fatal(err)
			}
			got, err := contract.MsgpackJSON(data)
			if err != nil {
				t.Fatal(err)
			}
			if string(got) != row.Encoded {
				t.Fatalf("got %s, want %s", got, row.Encoded)
			}
		})
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
}
