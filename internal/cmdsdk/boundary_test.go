package cmdsdk

import (
	"bytes"
	"context"
	"encoding/binary"
	"errors"
	"io"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/contract"
)

func TestExchangeStopsInputWhenServiceCompletes(t *testing.T) {
	c, _ := connected(t, fixture{})
	i, o, err := c.Invoke(t.Context(), invocation("short"))
	must(t, err)
	source := &waitingSource{}
	sink := &capture{}
	_, err = (Exchange{i, o}).Run(t.Context(), source, sink)
	must(t, err)
	if source.called {
		t.Fatal("read unrequested stdin")
	}
}

type waitingSource struct{ called bool }

func (s *waitingSource) Next(ctx context.Context) ([]byte, error) {
	s.called = true
	<-ctx.Done()
	return nil, ctx.Err()
}

func TestMetadataAndInputSizeRejectBeforeAllocation(t *testing.T) {
	for _, limit := range []uint32{cmdproto.MaxRecordBytes, cmdproto.MaxMetadataBytes} {
		var b [4]byte
		binary.BigEndian.PutUint32(b[:], limit+1)
		_, err := readChunk(bytes.NewReader(b[:]), limit)
		if !errors.Is(err, cmdproto.ErrTooLarge) {
			t.Fatal(err)
		}
	}
	_, err := readChunk(bytes.NewReader([]byte{0, 0}), 10)
	if !errors.Is(err, io.ErrUnexpectedEOF) {
		t.Fatal(err)
	}
}

// commandwire's tests hold the invocation rules; this pins that the service
// applies them to metadata before any handler runs.
func TestServiceRefusesInvalidInvocationMetadata(t *testing.T) {
	c, _ := connected(t, fixture{})
	valid, err := contract.EncodeJSON(invocation("short"))
	must(t, err)
	body := strings.Replace(string(valid), `"operation":"short"`, `"operation":""`, 1)
	if body == string(valid) {
		t.Fatal("mutation missed the operation")
	}
	b := make([]byte, 4)
	binary.BigEndian.PutUint32(b, uint32(len(body)))
	b = append(b, body...)
	_, _, err = c.invokeAt(t.Context(), cmdproto.InvokePath, b, true)
	if err == nil || !strings.Contains(err.Error(), "service rejected HTTP request with status 400") {
		t.Fatalf("accepted %s: %v", body, err)
	}
	short(t, c)
}
