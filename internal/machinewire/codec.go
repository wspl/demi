package machinewire

import (
	"fmt"

	"github.com/wspl/demi/internal/contract"
)

// MaxLineBytes is the longest line either end reads, its newline excluded. The largest real
// message is a few kilobytes.
const MaxLineBytes = 1 << 20

// Message identifies a message of this wire.
type Message interface{ WireMessage() }

// WireMessage identifies a message of the machine-manager socket.
func (MachineRequest) WireMessage() {}

// DecodeError reports a line that is not a message of this wire.
type DecodeError struct{ Err error }

// Error returns the failure message.
func (e *DecodeError) Error() string { return e.Err.Error() }

// Unwrap returns the underlying failure.
func (e *DecodeError) Unwrap() error { return e.Err }

// DecodeRequest decodes one request line, its newline removed.
func DecodeRequest(line []byte) (MachineRequest, error) {
	if len(line) > MaxLineBytes {
		return MachineRequest{}, &DecodeError{fmt.Errorf("machine line exceeds %d bytes", MaxLineBytes)}
	}
	value, err := DecodeMachineRequest(line)
	if err != nil {
		return MachineRequest{}, &DecodeError{err}
	}
	return value, nil
}

// DecodeResponse decodes one response line, its newline removed.
func DecodeResponse(line []byte) (MachineResponse, error) {
	if len(line) > MaxLineBytes {
		return nil, &DecodeError{fmt.Errorf("machine line exceeds %d bytes", MaxLineBytes)}
	}
	value, err := DecodeMachineResponse(line)
	if err != nil {
		return nil, &DecodeError{err}
	}
	return value, nil
}

// EncodeLine carries message as compact JSON followed by a newline.
func EncodeLine(message Message) ([]byte, error) {
	data, err := contract.EncodeJSON(message)
	if err != nil {
		return nil, fmt.Errorf("encode machine line: %w", err)
	}
	return append(data, '\n'), nil
}
