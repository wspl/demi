// Package agentproto defines the conversation WebSocket frames and transcript
// patches. It contains no session logic or transport.
package agentproto

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
)

type InvalidError = wire.InvalidError

// Failures is the map of provider diagnostics beside a transcript.
type Failures = map[string]core.ProviderFailureFacts

func Decode[T any](data []byte) (T, error) {
	var value T
	if err := json.Unmarshal(data, &value, wireOptions); err != nil {
		return value, wire.Refusal(err)
	}
	return value, check(value)
}
func Encode[T any](value T) ([]byte, error) {
	if err := check(value); err != nil {
		return nil, err
	}
	return json.Marshal(value, wireOptions)
}
func Validate[T any](value T) error { return check(value) }

// FrameError tells a connection whether to close (NotJSON) or answer invalid_frame.
type FrameError struct {
	NotJSON bool
	Err     error
}

func (e *FrameError) Error() string {
	if e.NotJSON {
		return "the message is not JSON: " + e.Err.Error()
	}
	return "invalid frame: " + e.Err.Error()
}
func (e *FrameError) Unwrap() error { return e.Err }
func DecodeClientFrame(data []byte) (ClientFrame, error) {
	frame, err := Decode[ClientFrame](data)
	if err != nil {
		var syntax *jsontext.SyntacticError
		return nil, &FrameError{NotJSON: errors.As(err, &syntax) && !errors.Is(err, jsontext.ErrDuplicateName), Err: err}
	}
	return frame, nil
}
func (v ClientFrameSend) check() error  { return checkNewContent(v.Content) }
func (v ClientFrameSteer) check() error { return checkNewContent(v.Content) }
func checkNewContent(content []ClientContent) error {
	var failures []error
	for i, part := range content {
		switch part.(type) {
		case ClientContentMedia, *ClientContentMedia, ClientContentAttachment, *ClientContentAttachment:
			failures = append(failures, &InvalidError{Path: "content" + wire.Index(i), Rule: "only an edit refers to the files its message holds"})
		}
	}
	return errors.Join(failures...)
}
func (v ClientFrameEditAndSend) check() error {
	for _, part := range v.Request.Content {
		switch part := part.(type) {
		case ClientContentText:
			if !core.IsBlank(part.Text) {
				return nil
			}
		case *ClientContentText:
			if part != nil && !core.IsBlank(part.Text) {
				return nil
			}
		default:
			return nil
		}
	}
	return &InvalidError{Path: "request.content", Rule: "a message must contain text or a file"}
}
