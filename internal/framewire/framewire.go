// Package framewire defines the conversation WebSocket's frames and validation.
package framewire

import (
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

// ClientFrame is a frame the web app sends. Its underlying contract is private
// so DecodeClientFrame can classify errors around the generated decoder.
type ClientFrame = clientFrame

// What the providers read out of the error blocks a frame carries, by block
// id (`backend.md` § Failure facts). Attached when the frame is sent, never
// stored.
type Failures map[core.BlockID]core.ProviderFailureFacts

// ErrNotJSON means the message is not JSON; the connection closes.
var ErrNotJSON = errors.New("the message is not JSON")

// ErrInvalidFrame means the message is JSON but not a valid frame; the
// connection stays open and the backend answers with invalid_frame.
var ErrInvalidFrame = errors.New("invalid frame")

// Decodes one message of the conversation WebSocket into a frame and checks
// it, before anything acts on it.
func DecodeClientFrame(data []byte) (ClientFrame, error) {
	if err := contract.CheckJSON(data); err != nil {
		if errors.Is(err, contract.ErrSyntax) {
			return nil, fmt.Errorf("%w: %w", ErrNotJSON, err)
		}
		return nil, fmt.Errorf("%w: %w", ErrInvalidFrame, err)
	}
	frame, err := decodeClientFrame(data)
	if err != nil {
		return nil, fmt.Errorf("%w: %w", ErrInvalidFrame, err)
	}
	return frame, nil
}

// ValidateClientFrame checks a frame constructed inside the process.
func ValidateClientFrame(frame ClientFrame) error { return validateClientFrame(frame) }

// validateSend restricts kept file references to edit frames.
func validateSend(v SendFrame) error { return validateNewContent(v.Content) }

// validateSteer restricts kept file references to edit frames.
func validateSteer(v SteerFrame) error { return validateNewContent(v.Content) }

// validateNewContent rejects references to files held by an edited message.
func validateNewContent(content []ClientContent) error {
	for i, part := range content {
		switch part.(type) {
		case *MediaContent, *AttachmentContent:
			return contract.At(
				fmt.Sprintf("content[%d]", i),
				errors.New("only an edit refers to the files its message holds"),
			)
		case *TextContent, *ReferenceContent, *UploadContent, *RemoteFileContent:
		}
	}
	return nil
}

// validateEditAndSend requires substance in the replacement message.
func validateEditAndSend(v EditAndSendFrame) error {
	for _, part := range v.Request.Content {
		switch part := part.(type) {
		case *TextContent:
			if !core.IsBlank(part.Text) {
				return nil
			}
		case *ReferenceContent, *UploadContent, *RemoteFileContent, *MediaContent, *AttachmentContent:
			return nil
		}
	}
	return contract.At("request.content", errors.New("a message must contain text or a file"))
}
