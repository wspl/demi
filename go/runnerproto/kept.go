package runnerproto

import (
	"fmt"

	"github.com/tinylib/msgp/msgp"
)

// OutputStream identifies a job's standard output or standard error.
//
//demi:enum
type OutputStream string

const (
	OutputStreamStdout OutputStream = "stdout"
	OutputStreamStderr OutputStream = "stderr"
)

// KeptRecord is one read or one gap in a job's stored output.
//
//demi:union tag=type
//demi:msgpack
type KeptRecord interface{ keptRecord() }

//demi:variant output
type KeptOutput struct {
	Stream OutputStream `json:"stream"`
	Bytes  []byte       `json:"bytes" msgpack:"bin"`
}

//demi:variant left_out
type KeptLeftOut struct {
	Bytes uint64 `json:"bytes"`
}

func (KeptOutput) keptRecord()  {}
func (KeptLeftOut) keptRecord() {}

// EncodeRecord writes a named record; records are concatenated in a kept file.
func EncodeRecord(record KeptRecord) ([]byte, error) { return EncodeKeptRecordMsgpack(record) }

// DecodeRecords validates a whole kept-output file, including its disk bound,
// nonempty reads and at most one gap.
func DecodeRecords(data []byte) ([]KeptRecord, error) {
	var records []KeptRecord
	kept, gaps := 0, 0
	for len(data) > 0 {
		rest, err := msgp.Skip(data)
		if err != nil {
			return nil, &InvalidError{Rule: "invalid kept record"}
		}
		size := len(data) - len(rest)
		record, err := DecodeKeptRecordMsgpack(data[:size])
		if err != nil {
			return nil, err
		}
		records = append(records, record)
		switch record := record.(type) {
		case KeptLeftOut:
			gaps++
		case KeptOutput:
			if len(record.Bytes) == 0 {
				return nil, fmt.Errorf("the kept output holds an empty read")
			}
			kept += size
		}
		if kept > JobKeptBytes {
			return nil, fmt.Errorf("the kept output's reads take more than %d bytes", JobKeptBytes)
		}
		if gaps > 1 {
			return nil, fmt.Errorf("the kept output leaves bytes out in more than one place")
		}
		data = rest
	}
	return records, nil
}
