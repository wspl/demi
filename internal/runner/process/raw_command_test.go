package process_test

import (
	"bytes"
	"testing"

	"github.com/wspl/demi/internal/runner/process"

	"github.com/wspl/demi/internal/contract"
)

func TestRawCommandBoundary(t *testing.T) {
	valid := []byte(`{"context":"0123456789abcdefABCDEF0123456789","root":"root","argv":["<>&"],"live":false}`)
	request, err := process.DecodeRawCommand(valid)
	if err != nil {
		t.Fatal(err)
	}
	encoded, err := contract.EncodeJSON(request)
	if err != nil || !bytes.Equal(encoded, valid) {
		t.Fatalf("wire %s: %v", encoded, err)
	}
	for _, input := range []string{
		`{"context":"bad","root":"root","argv":[],"live":false}`,
		`{"context":"0123456789abcdefABCDEF0123456789","root":"a/b","argv":[],"live":false}`,
		`{"context":"0123456789abcdefABCDEF0123456789","root":"root","argv":["a\u0000b"],"live":false}`,
		`{"context":"0123456789abcdefABCDEF0123456789","root":"root","argv":null,"live":false}`,
		`{"context":"0123456789abcdefABCDEF0123456789","root":"root","argv":[],"live":false,"extra":1}`,
		`{"context":"0123456789abcdefABCDEF0123456789","root":"root","argv":[],"live":false,"live":true}`,
	} {
		if _, err := process.DecodeRawCommand([]byte(input)); err == nil {
			t.Fatalf("accepted %s", input)
		}
	}
}
