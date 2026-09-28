package commandservice

import (
	"context"
	"fmt"
	"io"
)

// Exchange drives input and output concurrently until completion and response EOF,
// or the first failure. Each demand permits one nonempty Read or EOF; (0, nil)
// retries the same demand. The caller owns input and must unblock a pending Read
// after Exchange returns. That goroutine exits when Read returns and sends no more.
func Exchange(ctx context.Context, stream *Stream, input io.Reader, stdout, stderr io.Writer) (Completion, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	stop := context.AfterFunc(ctx, stream.Cancel)
	defer stop()
	demands := make(chan struct{}, 1)
	inputErrors := make(chan error, 1)
	go func() {
		var pendingError error
		for {
			select {
			case <-ctx.Done():
				return
			case <-demands:
			}
			buffer := make([]byte, MaxRecordBytes)
			for {
				if ctx.Err() != nil {
					return
				}
				count, err := 0, pendingError
				if err == nil {
					count, err = input.Read(buffer)
				}
				if ctx.Err() != nil {
					return
				}
				if count == 0 && err == nil {
					continue
				}
				if count > 0 {
					if writeErr := stream.Write(buffer[:count]); writeErr != nil {
						inputErrors <- fmt.Errorf("service: %w", writeErr)
						return
					}
					// A Read may return bytes and an error together. Each pull consumes
					// one input item; deliver that error on the next demand.
					pendingError = err
					break
				}
				if err == io.EOF {
					if endErr := stream.End(); endErr != nil {
						inputErrors <- fmt.Errorf("service: %w", endErr)
					}
					return
				}
				if err != nil {
					inputErrors <- fmt.Errorf("input: %w", err)
					return
				}
				break
			}
		}
	}()
	type response struct {
		record Record
		err    error
	}
	records := make(chan response)
	readerDone := make(chan struct{})
	go func() {
		defer close(readerDone)
		for {
			record, err := stream.Next()
			select {
			case records <- response{record, err}:
			case <-ctx.Done():
				return
			}
			if err != nil {
				return
			}
		}
	}()
	defer func() {
		cancel()
		stream.Cancel()
		<-readerDone
	}()
	var completion Completion
	for {
		select {
		case <-ctx.Done():
			return completion, fmt.Errorf("service: %w", ctx.Err())
		case err := <-inputErrors:
			return completion, err
		case item := <-records:
			if item.err == io.EOF {
				return completion, nil
			}
			if item.err != nil {
				return completion, fmt.Errorf("service: %w", item.err)
			}
			switch item.record.Kind {
			case Stdout, Stderr:
				writer := stdout
				if item.record.Kind == Stderr {
					writer = stderr
				}
				count, err := writer.Write(item.record.Data)
				if err == nil && count != len(item.record.Data) {
					err = io.ErrShortWrite
				}
				if err != nil {
					return completion, fmt.Errorf("output: %w", err)
				}
			case Completed:
				completion = item.record.Completion
			case InputPull:
				select {
				case demands <- struct{}{}:
				default:
					return completion, &InvalidError{Field: "input", Rule: "overlapping input demands"}
				}
			}
		}
	}
}
