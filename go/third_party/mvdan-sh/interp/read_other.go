//go:build !unix

package interp

import (
	"context"
	"time"
)

func cancellableReader(ctx context.Context, file stdinFile) (func([]byte) (int, error), func(), error) {
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() { file.SetReadDeadline(time.Now()); close(done) })
	cleanup := func() {
		if !stop() {
			<-done
			file.SetReadDeadline(time.Time{})
		}
	}
	return file.Read, cleanup, nil
}
