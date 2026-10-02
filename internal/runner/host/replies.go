package host

import (
	"context"
	"fmt"

	"github.com/wspl/demi/internal/runnerwire"
)

// sendFrame puts a generated Host frame on its owner's output queue.
func sendFrame(ctx context.Context, output chan<- []byte, message runnerwire.Outbound) error {
	frame, err := runnerwire.Encode(message)
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
func (s *Service) fsReply(ctx context.Context, id string, result runnerwire.FSResult, failure error) error {
	if failure != nil {
		return sendFrame(ctx, s.output, &runnerwire.FSError{ID: id, Code: ErrorCode(failure), Message: failure.Error()})
	}
	frame, err := runnerwire.Encode(&runnerwire.FSOK{ID: id, Result: result})
	if err == nil {
		frame, err = runnerwire.WithinLimit(frame, func(reason string) ([]byte, error) {
			code := "too_large"
			return runnerwire.Encode(&runnerwire.FSError{ID: id, Code: &code, Message: reason})
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
