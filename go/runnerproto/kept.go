package runnerproto

import (
	"fmt"

	"github.com/tinylib/msgp/msgp"
	"github.com/wspl/demi/go/internal/wire"
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
type KeptRecord interface{ keptRecord() }

type KeptOutput struct {
	Stream OutputStream
	Bytes  []byte
}

type KeptLeftOut struct {
	Bytes uint64
}

func (KeptOutput) keptRecord()  {}
func (KeptLeftOut) keptRecord() {}

// EncodeRecord writes Rust's positional record; records are concatenated in a kept file.
func EncodeRecord(record KeptRecord) ([]byte, error) {
	data := msgp.AppendMapHeader(nil, 1)
	switch record := record.(type) {
	case *KeptOutput:
		if record != nil {
			return EncodeRecord(*record)
		}
	case *KeptLeftOut:
		if record != nil {
			return EncodeRecord(*record)
		}
	case KeptOutput:
		if record.Stream != OutputStreamStdout && record.Stream != OutputStreamStderr {
			return nil, &InvalidError{Path: "stream", Rule: "unknown output stream"}
		}
		data = msgp.AppendString(data, "output")
		data = msgp.AppendArrayHeader(data, 2)
		data = msgp.AppendString(data, string(record.Stream))
		return msgp.AppendBytes(data, record.Bytes), nil
	case KeptLeftOut:
		data = msgp.AppendString(data, "left_out")
		return msgp.AppendUint64(data, record.Bytes), nil
	}
	return nil, &InvalidError{Rule: "unknown kept record"}
}

// decodeRecord reads the externally tagged tuple used by rmp_serde::to_vec.
func decodeRecord(data []byte) (KeptRecord, error) {
	count, rest, err := msgp.ReadMapHeaderBytes(data)
	if err != nil || count != 1 {
		return nil, &InvalidError{Rule: "expected one kept record variant"}
	}
	tag, rest, err := keptVariant(rest, "output", "left_out")
	if err != nil {
		return nil, err
	}
	if tag == "left_out" {
		value, err := wire.MPRead(rest, msgp.ReadUint64Bytes)
		return KeptLeftOut{Bytes: value}, err
	}
	count, rest, err = msgp.ReadArrayHeaderBytes(rest)
	if err != nil || count != 2 {
		return nil, &InvalidError{Rule: "expected output stream and bytes"}
	}
	// serde accepts a unit variant either as its identifier or a one-entry
	// map whose value is nil.
	unitMap := msgp.NextType(rest) == msgp.MapType
	if unitMap {
		count, rest, err = msgp.ReadMapHeaderBytes(rest)
		if err != nil || count != 1 {
			return nil, &InvalidError{Path: "stream", Rule: "expected one output stream variant"}
		}
	}
	stream, rest, err := keptVariant(rest, "stdout", "stderr")
	if err != nil {
		return nil, wire.In("stream", err)
	}
	if unitMap {
		rest, err = msgp.ReadNilBytes(rest)
		if err != nil {
			return nil, &InvalidError{Path: "stream", Rule: "expected nil unit variant"}
		}
	}
	data, err = wire.MPBytes(rest)
	if err != nil {
		return nil, wire.In("bytes", err)
	}
	return KeptOutput{Stream: OutputStream(stream), Bytes: data}, nil
}

// keptVariant reads serde's string, binary, or unsigned variant identifier.
func keptVariant(data []byte, names ...string) (string, []byte, error) {
	var name string
	var rest []byte
	var err error
	switch msgp.NextType(data) {
	case msgp.StrType:
		name, rest, err = msgp.ReadStringBytes(data)
	case msgp.BinType:
		var raw []byte
		raw, rest, err = msgp.ReadBytesBytes(data, nil)
		name = string(raw)
	default:
		// Positive fixints are unsigned to serde; explicitly signed prefixes are not.
		if len(data) > 0 && (data[0] < 0x80 || msgp.NextType(data) == msgp.UintType) {
			index, tail, err := msgp.ReadUint64Bytes(data)
			if err == nil && index < uint64(len(names)) {
				return names[index], tail, nil
			}
		}
		return "", nil, &InvalidError{Rule: "unknown kept record variant"}
	}
	if err == nil {
		for _, known := range names {
			if name == known {
				return name, rest, nil
			}
		}
	}
	return "", nil, &InvalidError{Rule: "unknown kept record variant"}
}

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
		record, err := decodeRecord(data[:size])
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
