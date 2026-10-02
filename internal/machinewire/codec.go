package machinewire

import (
	"fmt"

	"github.com/wspl/demi/internal/contract"
)

// The longest line either end reads, its newline excluded. The largest real
// message is a few kilobytes.
const MaxLineBytes = 1 << 20

// A message of this wire.
type Message interface{ WireMessage() }

func (MachineRequest) WireMessage() {}

// A line that is not a message of this wire.
type DecodeError struct{ Err error }

func (e *DecodeError) Error() string { return e.Err.Error() }
func (e *DecodeError) Unwrap() error { return e.Err }

// Decodes one request line, its newline removed.
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

// Decodes one response line, its newline removed.
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

// The line that carries `message`: compact JSON and a newline.
func EncodeLine(message Message) ([]byte, error) {
	data, err := contract.EncodeJSON(message)
	if err != nil {
		return nil, fmt.Errorf("encode machine line: %w", err)
	}
	return append(data, '\n'), nil
}
