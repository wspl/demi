package host

import (
	"context"
	"fmt"

	"github.com/wspl/demi/internal/runnerproto"
)

// sendFrame puts a generated Host frame on its owner's output queue.
func sendFrame(ctx context.Context, output chan<- []byte, message runnerproto.Outbound) error {
	frame, err := runnerproto.Encode(message)
	if err != nil {
		return fmt.Errorf("host reply encoding failed: %w", err)
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case output <- frame:
		return nil
	}
}

// fsReply applies the wire size limit to one filesystem result.
func (s *Service) fsReply(ctx context.Context, id string, result runnerproto.FSResult, failure error) error {
	if failure != nil {
		return sendFrame(
			ctx,
			s.output,
			&runnerproto.FSError{ID: id, Code: ErrorCode(failure), Message: failure.Error()},
		)
	}
	frame, err := runnerproto.Encode(&runnerproto.FSOK{ID: id, Result: result})
	if err == nil {
		frame, err = runnerproto.WithinLimit(frame, func(reason string) ([]byte, error) {
			code := "too_large"
			return runnerproto.Encode(&runnerproto.FSError{ID: id, Code: &code, Message: reason})
		})
	}
	if err != nil {
		return fmt.Errorf("filesystem response encoding failed: %w", err)
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case s.output <- frame:
		return nil
	}
}
