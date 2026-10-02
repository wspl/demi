package jobs

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/runnerwire"
)

// outputView tracks what the backend holds and the bounded newest bytes of a stream.
type outputView struct {
	stream                 runnerwire.OutputStream
	length, held, reported uint64
	tail                   []byte
	reportedAt             time.Time
}

func (v *outputView) write(bytes []byte) (uint64, []byte) {
	offset := v.length
	n := min(len(bytes), max(0, runnerwire.JobViewBytes-int(v.length)))
	v.length += uint64(len(bytes))
	if len(bytes) >= runnerwire.JobLiveBytes {
		v.tail = append(v.tail[:0], bytes[len(bytes)-runnerwire.JobLiveBytes:]...)
	} else {
		remove := max(0, len(v.tail)+len(bytes)-runnerwire.JobLiveBytes)
		v.tail = append(v.tail[:copy(v.tail, v.tail[remove:])], bytes...)
	}
	v.held += uint64(n)
	return offset, bytes[:n]
}
func (v *outputView) due(follow bool) time.Time {
	waiting := v.length > runnerwire.JobViewBytes && v.reported < v.length
	interval := runnerwire.JobGrowthInterval
	if follow {
		waiting = v.held < v.length
		interval = runnerwire.JobLiveInterval
	}
	if !waiting {
		return time.Time{}
	}
	if v.reportedAt.IsZero() {
		return time.Now()
	}
	return v.reportedAt.Add(interval)
}
func (v *outputView) frame(job string, follow bool) ([]byte, error) {
	var offset uint64
	if follow {
		offset = max(v.held, uint64(max(0, int64(v.length)-runnerwire.JobLiveBytes)))
	} else {
		offset = max(runnerwire.JobViewBytes, uint64(max(0, int64(v.length)-runnerwire.JobViewBytes)))
	}
	start := v.length - uint64(len(v.tail))
	return runnerwire.Encode(&runnerwire.JobOutput{JobID: job, Stream: v.stream, Offset: offset, Bytes: append([]byte{}, v.tail[offset-start:]...)})
}
func (v *outputView) sent(follow bool) {
	v.reported = v.length
	v.reportedAt = time.Now()
	if follow {
		v.held = v.length
	}
}
func (v *outputView) beyond(job string, follow, force bool, output chan<- []byte) error {
	due := v.due(follow)
	if due.IsZero() || (!force && due.After(time.Now())) {
		return nil
	}
	frame, err := v.frame(job, follow)
	if err != nil {
		return err
	}
	select {
	case output <- frame:
		v.sent(follow)
	default:
		v.reportedAt = time.Now()
	}
	return nil
}
func (v *outputView) last(ctx context.Context, job string, follow bool, output chan<- []byte) error {
	if v.due(follow).IsZero() {
		return nil
	}
	frame, err := v.frame(job, follow)
	if err != nil {
		return err
	}
	select {
	case <-ctx.Done():
		return nil
	case output <- frame:
		v.sent(follow)
		return nil
	}
}
