package commandservice

import (
	"encoding/binary"
	"errors"
	"io"
)

// A RecordKind says what a response record carries.
type RecordKind uint8

// The kinds of response record.
const (
	// RecordStdout carries raw standard output bytes.
	RecordStdout RecordKind = 1
	// RecordStderr carries raw standard error bytes.
	RecordStderr RecordKind = 2
	// RecordCompletion carries the invocation's completion, which ends it.
	RecordCompletion RecordKind = 3
	// RecordInputPull asks the caller for exactly one input chunk, or its
	// end.
	RecordInputPull RecordKind = 4
)

// A Record is one unit of a response: a kind byte, a four-byte big-endian
// payload length, and the payload. DATA frame boundaries are unrelated to
// record boundaries.
type Record struct {
	Kind RecordKind
	// Data is the payload of a standard output or standard error record.
	Data []byte
	// Completion is the payload of a completion record.
	Completion Completion
}

const recordHeaderBytes = 5

// MarshalBinary returns the record's bytes on the wire. It refuses a payload
// over [MaxRecordBytes].
func (r Record) MarshalBinary() ([]byte, error) {
	var payload []byte
	switch r.Kind {
	case RecordStdout, RecordStderr:
		payload = r.Data
	case RecordCompletion:
		var err error
		payload, err = Encode(r.Completion)
		if err != nil {
			return nil, err
		}
	case RecordInputPull:
		// A pull carries no payload.
	default:
		return nil, ErrUnknownRecord
	}
	if len(payload) > MaxRecordBytes {
		return nil, ErrTooLarge
	}
	data := make([]byte, recordHeaderBytes+len(payload))
	data[0] = byte(r.Kind)
	binary.BigEndian.PutUint32(data[1:], uint32(len(payload)))
	copy(data[recordHeaderBytes:], payload)
	return data, nil
}

// A RecordReader reads the records of one response. It buffers at most one
// record, and refuses one whose length is over the limit before its payload
// arrives.
type RecordReader struct {
	reader    io.Reader
	completed bool
	err       error
}

// NewRecordReader returns a reader of the records that r carries.
func NewRecordReader(r io.Reader) *RecordReader {
	return &RecordReader{reader: r}
}

// Next returns the next record. After the completion record it returns
// [io.EOF] when the response ends, and [ErrAfterCompletion] when data follows.
// It returns [ErrIncomplete] when the response ends before its completion,
// [ErrUnknownRecord] for a kind the wire does not define, [ErrTooLarge] for a
// payload over [MaxRecordBytes], and an [*InvalidError] for a payload that
// breaks its kind's rules. An error is final: later calls return it again.
func (r *RecordReader) Next() (Record, error) {
	if r.err != nil {
		return Record{}, r.err
	}
	record, err := r.next()
	if err != nil {
		r.err = err
		return Record{}, err
	}
	return record, nil
}

func (r *RecordReader) next() (Record, error) {
	if r.completed {
		var extra [1]byte
		if _, err := io.ReadFull(r.reader, extra[:]); err != nil {
			return Record{}, err
		}
		return Record{}, ErrAfterCompletion
	}
	var header [recordHeaderBytes]byte
	if _, err := io.ReadFull(r.reader, header[:]); err != nil {
		return Record{}, unfinished(err)
	}
	kind := RecordKind(header[0])
	if kind < RecordStdout || kind > RecordInputPull {
		return Record{}, ErrUnknownRecord
	}
	length := binary.BigEndian.Uint32(header[1:])
	if length > MaxRecordBytes {
		return Record{}, ErrTooLarge
	}
	payload := make([]byte, length)
	if _, err := io.ReadFull(r.reader, payload); err != nil {
		return Record{}, unfinished(err)
	}
	switch kind {
	case RecordCompletion:
		completion, err := Decode[Completion](payload)
		if err != nil {
			return Record{}, err
		}
		r.completed = true
		return Record{Kind: kind, Completion: completion}, nil
	case RecordInputPull:
		if len(payload) > 0 {
			return Record{}, &InvalidError{Reason: "an input pull record carries no payload"}
		}
		return Record{Kind: kind}, nil
	default:
		return Record{Kind: kind, Data: payload}, nil
	}
}

// unfinished turns the end of a stream in the middle of a unit of the wire
// into [ErrIncomplete].
func unfinished(err error) error {
	if errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) {
		return ErrIncomplete
	}
	return err
}

// EncodeInput frames one input chunk for the wire: a four-byte big-endian
// length, then the payload. A chunk is at most [MaxRecordBytes]; each input
// pull of the service permits one chunk, and the end of the request stream
// stands for the end of input.
func EncodeInput(chunk []byte) ([]byte, error) {
	return frame(chunk, MaxRecordBytes)
}

// frame returns payload behind its length as a four-byte big-endian prefix, or
// [ErrTooLarge] when the payload is over limit.
func frame(payload []byte, limit int) ([]byte, error) {
	if len(payload) > limit {
		return nil, ErrTooLarge
	}
	data := make([]byte, 4+len(payload))
	binary.BigEndian.PutUint32(data, uint32(len(payload)))
	copy(data[4:], payload)
	return data, nil
}

// readFrame reads one frame written by [frame]. It returns [io.EOF] when the
// stream ends before the frame begins, [ErrIncomplete] when it ends inside the
// frame, and [ErrTooLarge] as soon as the length is over limit, before any
// payload is read.
func readFrame(r io.Reader, limit int) ([]byte, error) {
	var prefix [4]byte
	if _, err := io.ReadFull(r, prefix[:]); err != nil {
		if errors.Is(err, io.EOF) {
			return nil, io.EOF
		}
		return nil, unfinished(err)
	}
	length := binary.BigEndian.Uint32(prefix[:])
	if length > uint32(limit) {
		return nil, ErrTooLarge
	}
	payload := make([]byte, length)
	if _, err := io.ReadFull(r, payload); err != nil {
		return nil, unfinished(err)
	}
	return payload, nil
}
