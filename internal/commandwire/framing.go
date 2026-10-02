package commandwire

import (
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
)

// Wire limits and paths are shared by the command client and service.
const (
	MaxMetadataBytes       = 256 * 1024
	MaxRecordBytes         = 64 * 1024
	MaxNumbers             = 16
	CommandLocaleLanguages = 16
	ConversationNameChars  = 64
	InfoPath               = "/v1/info"
	InvokePath             = "/v1/invoke"
	ConversationPath       = "/v1/conversation"
	NumbersPath            = "/v1/numbers"
	ArtifactsPath          = "/v1/artifacts"
	ShutdownPath           = "/v1/shutdown"
)

// Framing errors can be inspected with errors.Is.
var (
	ErrTooLarge        = errors.New("command protocol payload exceeds limit")
	ErrMissingTarget   = errors.New("package has no artifact for target")
	ErrUnknownRecord   = errors.New("unknown response record kind")
	ErrAfterCompletion = errors.New("response contains data after completion")
	ErrIncomplete      = errors.New("response ended without complete final status")
	ErrInvalidRecord   = errors.New("invalid response record payload")
)

// Record is one response frame; only its variant's payload is present.
//
//sumtype:decl
type Record interface{ record() }

// Stdout is a bounded raw standard output chunk.
type Stdout []byte

func (Stdout) record() {}

// Stderr is a bounded raw standard error chunk.
type Stderr []byte

func (Stderr) record() {}

// Completed ends the response with a command completion.
type Completed struct {
	Completion Completion `json:"completion"`
}

func (Completed) record() {}

// InputPull permits exactly one stdin chunk or EOF.
type InputPull struct{}

func (InputPull) record() {}

// EncodeRecord frames one response record.
func EncodeRecord(record Record) ([]byte, error) {
	var kind byte
	var payload []byte
	switch r := record.(type) {
	case Stdout:
		kind, payload = 1, []byte(r)
	case Stderr:
		kind, payload = 2, []byte(r)
	case Completed:
		kind = 3
		var err error
		payload, err = json.Marshal(r.Completion)
		if err != nil {
			return nil, fmt.Errorf("encode completion: %w", err)
		}
	case InputPull:
		kind = 4
	default:
		return nil, ErrUnknownRecord
	}
	if len(payload) > MaxRecordBytes {
		return nil, ErrTooLarge
	}
	result := make([]byte, 5+len(payload))
	result[0] = kind
	binary.BigEndian.PutUint32(result[1:5], uint32(len(payload)))
	copy(result[5:], payload)
	return result, nil
}

// The metadata that opens an invocation stream: the native protocol's
// [`Invocation`], or the local command client's [`LocalInvocation`], which
// shares its framing, input demand and completion.
type Metadata interface {
	metadata()
	Validate() error
}

func (Invocation) metadata()      {}
func (LocalInvocation) metadata() {}

// EncodeMetadata validates and length-prefixes command invocation metadata.
func EncodeMetadata(value Metadata) ([]byte, error) {
	if err := value.Validate(); err != nil {
		return nil, fmt.Errorf("metadata: %w", err)
	}
	return encodeMetadata(value)
}

// EncodeConversationRequest validates and frames a conversation lifecycle request.
func EncodeConversationRequest(value ConversationRequest) ([]byte, error) {
	if err := ValidateConversationRequest(value); err != nil {
		return nil, fmt.Errorf("metadata: %w", err)
	}
	return encodeMetadata(value)
}

// Encode frames the metadata opening a numbers or artifacts stream.
func (value StreamOpen) Encode() ([]byte, error) {
	if err := value.Validate(); err != nil {
		return nil, fmt.Errorf("metadata: %w", err)
	}
	return encodeMetadata(value)
}

// encodeMetadata applies the shared command metadata length prefix and limit.
func encodeMetadata(value any) ([]byte, error) {
	data, err := json.Marshal(value)
	if err != nil {
		return nil, fmt.Errorf("encode metadata: %w", err)
	}
	if len(data) > MaxMetadataBytes {
		return nil, ErrTooLarge
	}
	result := make([]byte, 4+len(data))
	binary.BigEndian.PutUint32(result[:4], uint32(len(data)))
	copy(result[4:], data)
	return result, nil
}

// EncodeInput preserves one stdin chunk boundary across transport fragments.
func EncodeInput(data []byte) ([]byte, error) {
	if len(data) > MaxRecordBytes {
		return nil, ErrTooLarge
	}
	result := make([]byte, 4+len(data))
	binary.BigEndian.PutUint32(result[:4], uint32(len(data)))
	copy(result[4:], data)
	return result, nil
}

// RecordDecoder buffers at most one bounded frame. Its zero value is ready.
// Decode consumes only the bytes needed for that frame; callers retain the rest.
type RecordDecoder struct {
	pending   []byte
	completed bool
}

// Decode returns a record or nil if more input is needed, and the consumed byte count.
func (d *RecordDecoder) Decode(input []byte) (Record, int, error) {
	if d.completed {
		if len(input) != 0 {
			return nil, 0, ErrAfterCompletion
		}
		return nil, 0, nil
	}
	consumed := d.take(input, 5)
	if len(d.pending) < 5 {
		return nil, consumed, nil
	}
	kind := d.pending[0]
	if kind < 1 || kind > 4 {
		return nil, consumed, ErrUnknownRecord
	}
	length := binary.BigEndian.Uint32(d.pending[1:5])
	if length > MaxRecordBytes {
		return nil, consumed, ErrTooLarge
	}
	consumed += d.take(input[consumed:], 5+int(length))
	if len(d.pending) < 5+int(length) {
		return nil, consumed, nil
	}
	payload := d.pending[5:]
	d.pending = nil
	switch kind {
	case 1:
		return Stdout(payload), consumed, nil
	case 2:
		return Stderr(payload), consumed, nil
	case 3:
		value, err := DecodeCompletion(payload)
		if err != nil {
			return nil, consumed, fmt.Errorf("decode completion: %w", err)
		}
		d.completed = true
		return Completed{Completion: value}, consumed, nil
	case 4:
		if len(payload) != 0 {
			return nil, consumed, ErrInvalidRecord
		}
		return InputPull{}, consumed, nil
	}
	return nil, consumed, ErrUnknownRecord
}

// Finish rejects a truncated stream or one lacking completion.
func (d *RecordDecoder) Finish() error {
	if !d.completed || len(d.pending) != 0 {
		return ErrIncomplete
	}
	return nil
}

// take buffers just the missing portion of a command response frame.
func (d *RecordDecoder) take(input []byte, target int) int {
	n := min(max(target-len(d.pending), 0), len(input))
	d.pending = append(d.pending, input[:n]...)
	return n
}
