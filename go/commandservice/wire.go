package commandservice

import (
	"context"
	"encoding/binary"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"reflect"

	"github.com/google/jsonschema-go/jsonschema"
)

var (
	// ErrTooLarge reports a frame exceeding its protocol limit.
	ErrTooLarge = errors.New("command protocol payload exceeds limit")
	// ErrInvalid reports a wire value that violates its contract.
	ErrInvalid = errors.New("invalid command protocol value")
	// ErrUnknownRecord reports an unrecognized response-record kind.
	ErrUnknownRecord = errors.New("unknown response record kind")
	// ErrAfterCompletion reports response bytes after the final completion.
	ErrAfterCompletion = errors.New("response contains data after completion")
	// ErrIncomplete reports a truncated response or a missing completion.
	ErrIncomplete = errors.New("response ended without complete final status")
	// ErrCancelled identifies command cancellation.
	ErrCancelled = context.Canceled
	// ErrCancellationDeadline requires retiring a process whose handler or Closer did not stop.
	ErrCancellationDeadline = errors.New("handler exceeded cancellation deadline; retire the service process")
	// ErrConversationCleanup requires retiring a service after failed conversation cleanup.
	ErrConversationCleanup = errors.New("conversation cleanup failed; retire the service process")
)

// InvalidError names a command wire field and the rule it violates.
type InvalidError struct{ Field, Rule string }

// Error returns the protocol failure description.
func (err *InvalidError) Error() string {
	return fmt.Sprintf("%s: %s: %s", ErrInvalid, err.Field, err.Rule)
}

// Unwrap allows errors.Is to recognize ErrInvalid.
func (err *InvalidError) Unwrap() error { return ErrInvalid }

// RejectedError reports an HTTP rejection before command execution.
type RejectedError struct{ Status int }

// Error returns the protocol failure description.
func (err *RejectedError) Error() string {
	return fmt.Sprintf("service rejected HTTP status %d", err.Status)
}

func schemaFor[T WireValue]() (*jsonschema.Resolved, error) {
	schemas, err := wireSchemas()
	if err != nil {
		return nil, err
	}
	t := reflect.TypeFor[T]()
	if t.Kind() == reflect.Pointer {
		t = t.Elem()
	}
	schema, ok := schemas[t]
	if !ok {
		return nil, &InvalidError{Field: "type", Rule: "SDK wire schema is missing"}
	}
	return schema, nil
}

// Decode validates a wire value’s schema, decodes it without changing opaque JSON, and runs
// its nested checks. Framing supplies any size limit.
func Decode[T WireValue](b []byte) (T, error) {
	var v T
	s, err := schemaFor[T]()
	if err != nil {
		return v, err
	}
	var raw any
	if err = json.Unmarshal(b, &raw); err != nil {
		return v, &InvalidError{Field: "JSON", Rule: err.Error()}
	}
	if raw == nil {
		return v, &InvalidError{Field: "value", Rule: "must not be null"}
	}
	if err = s.Validate(raw); err != nil {
		return v, &InvalidError{Field: "value", Rule: err.Error()}
	}
	if err = json.Unmarshal(b, &v); err != nil {
		return v, &InvalidError{Field: "value", Rule: err.Error()}
	}
	return v, v.validate()
}

// Encode validates a wire value and encodes it with deterministic map ordering. Framing
// supplies any size limit.
func Encode[T WireValue](v T) ([]byte, error) {
	b, err := json.Marshal(v, json.Deterministic(true))
	if err != nil {
		return nil, err
	}
	if _, err = Decode[T](b); err != nil {
		return nil, err
	}
	return b, nil
}

func frame(b []byte, limit int) ([]byte, error) {
	if len(b) > limit {
		return nil, ErrTooLarge
	}
	out := make([]byte, 4+len(b))
	binary.BigEndian.PutUint32(out, uint32(len(b)))
	copy(out[4:], b)
	return out, nil
}

func readFrame(r io.Reader, limit int) ([]byte, error) {
	var head [4]byte
	if _, err := io.ReadFull(r, head[:]); err != nil {
		return nil, err
	}
	n := binary.BigEndian.Uint32(head[:])
	if n > uint32(limit) {
		return nil, ErrTooLarge
	}
	b := make([]byte, n)
	_, err := io.ReadFull(r, b)
	return b, err
}

// EncodeInput frames one input chunk with its bounded four-byte length prefix.
func EncodeInput(b []byte) ([]byte, error) { return frame(b, MaxRecordBytes) }

// EncodeMetadata validates and frames one metadata value, limited to MaxMetadataBytes.
func EncodeMetadata[T WireValue](v T) ([]byte, error) {
	b, err := Encode(v)
	if err != nil {
		return nil, err
	}
	return frame(b, MaxMetadataBytes)
}

// RecordKind identifies a response record’s payload.
type RecordKind byte

const (
	// Stdout carries standard-output bytes.
	Stdout RecordKind = 1
	// Stderr carries standard-error bytes.
	Stderr RecordKind = 2
	// Completed carries the final command completion.
	Completed RecordKind = 3
	// InputPull permits exactly one input chunk or EOF.
	InputPull RecordKind = 4
)

// Record carries stdout, stderr, completion, or one input demand.
type Record struct {
	Kind       RecordKind
	Data       []byte
	Completion Completion
}

// Encode frames this response record after validating its kind and payload.
func (r Record) Encode() ([]byte, error) {
	b := r.Data
	var err error
	switch r.Kind {
	case Stdout, Stderr:
	case Completed:
		b, err = Encode(r.Completion)
	case InputPull:
		if len(b) != 0 {
			return nil, ErrInvalid
		}
	default:
		return nil, ErrUnknownRecord
	}
	if err != nil {
		return nil, err
	}
	f, err := frame(b, MaxRecordBytes)
	if err != nil {
		return nil, err
	}
	return append([]byte{byte(r.Kind)}, f...), nil
}

// RecordDecoder reads bounded records independently of transport fragmentation and requires
// a final completion.
type RecordDecoder struct {
	Reader    io.Reader
	completed bool
}

// Next reads one bounded record and rejects truncation or bytes after completion.
func (d *RecordDecoder) Next() (Record, error) {
	var r Record
	var header [5]byte
	if d.completed {
		_, err := io.ReadFull(d.Reader, header[:1])
		if err == io.EOF {
			return r, io.EOF
		}
		if err != nil {
			return r, err
		}
		return r, ErrAfterCompletion
	}
	if _, err := io.ReadFull(d.Reader, header[:]); err != nil {
		if errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) {
			return r, ErrIncomplete
		}
		return r, err
	}
	r.Kind = RecordKind(header[0])
	if r.Kind < Stdout || r.Kind > InputPull {
		return r, ErrUnknownRecord
	}
	length := binary.BigEndian.Uint32(header[1:])
	if length > MaxRecordBytes {
		return r, ErrTooLarge
	}
	r.Data = make([]byte, length)
	_, err := io.ReadFull(d.Reader, r.Data)
	if err != nil {
		if errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) {
			return r, ErrIncomplete
		}
		return r, err
	}
	if r.Kind == Completed {
		r.Completion, err = Decode[Completion](r.Data)
		if err == nil {
			d.completed = true
		}
	}
	if r.Kind == InputPull && len(r.Data) != 0 {
		return r, ErrInvalid
	}
	return r, err
}
