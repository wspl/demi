package cmdpkgs

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"strings"
	"time"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
)

// A successful conversation release has an empty acknowledgement.
// +demi:root
type releaseAcknowledgement struct{}

func serviceStatus(ctx context.Context, client *cmdsdk.Client) (bool, error) {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	answer, err := conversationCall(ctx, client, &cmdproto.ConversationQuery{})
	if err != nil {
		if errors.Is(ctx.Err(), context.DeadlineExceeded) {
			return false, &conversationError{message: "no answer within 5 seconds", cause: err}
		}
		return false, err
	}
	status, err := cmdproto.DecodeConversationStatus(answer)
	if err != nil {
		return false, err
	}
	return len(status.Conversations) != 0, nil
}

func releaseConversation(ctx context.Context, client *cmdsdk.Client, conversation string) error {
	ctx, cancel := context.WithTimeout(ctx, 360*time.Second)
	defer cancel()
	answer, err := conversationCall(ctx, client, &cmdproto.ConversationRelease{Conversation: conversation})
	if err != nil {
		if errors.Is(ctx.Err(), context.DeadlineExceeded) {
			return &conversationError{message: "the conversation release did not finish in time", cause: err}
		}
		return err
	}
	if _, err := decodeReleaseAcknowledgement(answer); err != nil {
		return errors.New("invalid conversation release acknowledgement")
	}
	return nil
}

func conversationCall(
	ctx context.Context,
	client *cmdsdk.Client,
	request cmdproto.ConversationRequest,
) ([]byte, error) {
	input, output, err := client.Conversation(ctx, request)
	if err != nil {
		return nil, err
	}
	defer input.Cancel()
	var data []byte
	completed := false
	for {
		record, err := output.Next(ctx)
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return nil, err
		}
		switch v := record.(type) {
		case cmdproto.Stdout:
			if len(data)+len(v) > 1024*1024 {
				return nil, errors.New("the conversation answer exceeds 1 MiB")
			}
			data = append(data, v...)
		case cmdproto.Stderr:
			slog.Warn(string(v))
		case cmdproto.Completed:
			if v.Completion.ExitCode != 0 || v.Completion.Error != nil {
				return nil, errors.New("the conversation operation failed")
			}
			completed = true
		case cmdproto.InputPull:
			return nil, errors.New("the conversation operation failed")
		}
	}
	if !completed {
		return nil, errors.New("the conversation operation has no completion")
	}
	return data, nil
}

// conversationError preserves the wire-facing diagnostic and its underlying cause.
type conversationError struct {
	message string
	cause   error
}

func (e *conversationError) Error() string {
	return e.message
}

func (e *conversationError) Unwrap() error {
	return e.cause
}

func releaseFailures(failures []error) error {
	if len(failures) == 0 {
		return nil
	}
	messages := make([]string, len(failures))
	for i, err := range failures {
		messages[i] = err.Error()
	}
	return &conversationError{message: strings.Join(messages, "; "), cause: errors.Join(failures...)}
}
