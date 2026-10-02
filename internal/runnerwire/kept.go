package runnerwire

import (
	"bytes"
	"errors"
	"fmt"

	"github.com/vmihailenco/msgpack/v5"
)

// The most bytes of records of reads a kept output holds; below it, it holds
// every read.
const JobKeptBytes = 16 * 1024 * 1024

// Beyond [JobKeptBytes], the bytes of records kept at the start, and at
// most those kept at the end.
const JobKeptPartBytes = JobKeptBytes / 2

// The most bytes a kept output takes: [JobKeptBytes] of reads, and the
// record between its parts that counts the bytes left out.
const JobKeptReadBytes = JobKeptBytes + 32

// One record of a kept output.
// +demi:root
// +demi:union tag=type
// +demi:msgpack tuple
//
//sumtype:decl
type KeptRecord interface{ keptRecord() }

// +demi:enum stdout stderr
type OutputStream string

// Streams retained in job output.
const (
	Stdout OutputStream = "stdout"
	Stderr OutputStream = "stderr"
)

// Bytes that travel as MessagePack `bin`.
// +demi:base64
type WireBytes []byte

// One read of the job's stdout or stderr.
// +demi:variant KeptRecord output
type KeptOutput struct {
	Stream OutputStream `json:"stream"`
	Bytes  WireBytes    `json:"bytes"`
}

// How many bytes of output the kept output leaves out here, between its
// first part and its last.
// +demi:variant KeptRecord left_out
type KeptLeftOut struct {
	Bytes uint64 `json:"bytes"`
}

// EncodeRecord returns the record's bytes, to append to a kept output.
func EncodeRecord(record KeptRecord) ([]byte, error) {
	return EncodeKeptRecordMsgpack(record)
}

// DecodeRecords reads a kept output, refusing empty reads, more than one
// gap, and reads whose encoded size exceeds JobKeptBytes.
func DecodeRecords(data []byte) ([]KeptRecord, error) {
	reader := bytes.NewReader(data)
	decoder := msgpack.NewDecoder(reader)
	records := make([]KeptRecord, 0)
	kept := 0
	gaps := 0
	for reader.Len() > 0 {
		record, size, err := readKeptRecord(decoder)
		if err != nil {
			return nil, fmt.Errorf("kept record: %w", err)
		}
		switch record := record.(type) {
		case *KeptOutput:
			if len(record.Bytes) == 0 {
				return nil, errors.New("the kept output holds an empty read")
			}
			kept += size
		case *KeptLeftOut:
			gaps++
		}
		if kept > JobKeptBytes {
			return nil, fmt.Errorf("the kept output's reads take more than %d bytes", JobKeptBytes)
		}
		if gaps > 1 {
			return nil, errors.New("the kept output leaves bytes out in more than one place")
		}
		records = append(records, record)
	}
	return records, nil
}

// readKeptRecord frames one stored record and passes it to its generated decoder.
func readKeptRecord(decoder *msgpack.Decoder) (KeptRecord, int, error) {
	raw, err := decoder.DecodeRaw()
	if err != nil {
		return nil, 0, err
	}
	record, err := DecodeKeptRecordMsgpack(raw)
	return record, len(raw), err
}
