package commandservice_test

import (
	"bytes"
	"errors"
	"io"
	"reflect"
	"testing"

	"github.com/wspl/demi/go/commandservice"
)

// chunked is a reader that returns at most size bytes at a time, as DATA
// frames of any size do.
type chunked struct {
	data []byte
	size int
}

func (c *chunked) Read(p []byte) (int, error) {
	if len(c.data) == 0 {
		return 0, io.EOF
	}
	n := min(len(p), c.size, len(c.data))
	copy(p, c.data[:n])
	c.data = c.data[n:]
	return n, nil
}

func marshal(t *testing.T, records ...commandservice.Record) []byte {
	t.Helper()
	var wire []byte
	for _, record := range records {
		bytes, err := record.MarshalBinary()
		if err != nil {
			t.Fatal(err)
		}
		wire = append(wire, bytes...)
	}
	return wire
}

func TestRecordsDecodeAtEveryFragmentationBoundaryAndKeepBinaryPayloads(t *testing.T) {
	want := []commandservice.Record{
		{Kind: commandservice.RecordInputPull},
		{Kind: commandservice.RecordStdout, Data: []byte{0, 255, 13, 10}},
		{Kind: commandservice.RecordStderr, Data: []byte("diagnostic")},
		{Kind: commandservice.RecordCompletion, Completion: commandservice.Completion{ExitCode: 7}},
	}
	wire := marshal(t, want...)
	for size := 1; size <= len(wire); size++ {
		reader := commandservice.NewRecordReader(&chunked{data: wire, size: size})
		var got []commandservice.Record
		for {
			record, err := reader.Next()
			if err == io.EOF {
				break
			}
			if err != nil {
				t.Fatalf("size %d: %v", size, err)
			}
			got = append(got, record)
		}
		if !reflect.DeepEqual(got, want) {
			t.Fatalf("size %d: records = %+v, want %+v", size, got, want)
		}
	}
}

// A record's payload, and an input chunk, is at most 64 KiB: one of exactly
// that size goes through, and one byte more is refused, by the reader before its
// payload arrives.
func TestARecordOverTheLimitIsRefusedBeforeItsPayloadArrives(t *testing.T) {
	// Only the header arrives: a reader that waited for the payload would
	// report an incomplete response instead. Its length is 65537.
	header := []byte{1, 0, 1, 0, 1}
	_, err := commandservice.NewRecordReader(bytes.NewReader(header)).Next()
	if !errors.Is(err, commandservice.ErrTooLarge) {
		t.Errorf("error = %v, want ErrTooLarge", err)
	}
	within := make([]byte, 65536)
	over := make([]byte, 65537)
	if _, err := (commandservice.Record{Kind: commandservice.RecordStdout, Data: over}).MarshalBinary(); !errors.Is(err, commandservice.ErrTooLarge) {
		t.Errorf("marshaling: error = %v, want ErrTooLarge", err)
	}
	if _, err := commandservice.EncodeInput(over); !errors.Is(err, commandservice.ErrTooLarge) {
		t.Errorf("input: error = %v, want ErrTooLarge", err)
	}
	record, err := commandservice.NewRecordReader(bytes.NewReader(marshal(t, commandservice.Record{Kind: commandservice.RecordStdout, Data: within}))).Next()
	if err != nil || len(record.Data) != 65536 {
		t.Errorf("a record of 65536 bytes: %d bytes, %v", len(record.Data), err)
	}
	if _, err := commandservice.EncodeInput(within); err != nil {
		t.Errorf("an input chunk of 65536 bytes: %v", err)
	}
}

func TestACompletionIsRequiredAndNothingFollowsIt(t *testing.T) {
	partial := marshal(t, commandservice.Record{Kind: commandservice.RecordStdout, Data: []byte("partial")})
	reader := commandservice.NewRecordReader(bytes.NewReader(partial))
	if _, err := reader.Next(); err != nil {
		t.Fatal(err)
	}
	if _, err := reader.Next(); !errors.Is(err, commandservice.ErrIncomplete) {
		t.Errorf("a response without a completion: error = %v, want ErrIncomplete", err)
	}
	// A response that stops inside a record is incomplete too.
	if _, err := commandservice.NewRecordReader(bytes.NewReader(partial[:len(partial)-1])).Next(); !errors.Is(err, commandservice.ErrIncomplete) {
		t.Errorf("a record cut short: error = %v, want ErrIncomplete", err)
	}
	completion := marshal(t, commandservice.Record{Kind: commandservice.RecordCompletion})
	reader = commandservice.NewRecordReader(bytes.NewReader(append(completion, completion...)))
	if _, err := reader.Next(); err != nil {
		t.Fatal(err)
	}
	if _, err := reader.Next(); !errors.Is(err, commandservice.ErrAfterCompletion) {
		t.Errorf("a record after the completion: error = %v, want ErrAfterCompletion", err)
	}
	// An error is final.
	if _, err := reader.Next(); !errors.Is(err, commandservice.ErrAfterCompletion) {
		t.Errorf("after an error: %v", err)
	}
}

func TestARecordThatBreaksItsKindsRulesIsRefused(t *testing.T) {
	record := func(kind byte, payload string) []byte {
		return append([]byte{kind, 0, 0, 0, byte(len(payload))}, payload...)
	}
	for name, wire := range map[string][]byte{
		"exit code below 0":    record(3, `{"exitCode":-1}`),
		"exit code above 255":  record(3, `{"exitCode":256}`),
		"completion not JSON":  record(3, `exit`),
		"completion with null": record(3, `{"exitCode":0,"error":null}`),
		"pull with a payload":  record(4, `x`),
		"kind 0":               record(0, ``),
		"kind 5":               record(5, ``),
	} {
		_, err := commandservice.NewRecordReader(bytes.NewReader(wire)).Next()
		if err == nil {
			t.Errorf("%s: the record was accepted", name)
		}
	}
	_, err := commandservice.NewRecordReader(bytes.NewReader(record(9, ""))).Next()
	if !errors.Is(err, commandservice.ErrUnknownRecord) {
		t.Errorf("an unknown kind: error = %v, want ErrUnknownRecord", err)
	}
	var invalid *commandservice.InvalidError
	_, err = commandservice.NewRecordReader(bytes.NewReader(record(3, `{"exitCode":256}`))).Next()
	if !errors.As(err, &invalid) {
		t.Errorf("an invalid completion: error = %v, want an InvalidError", err)
	}
}
