package jobs

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sync"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runnerwire"
)

// KeptOutput keeps bounded output records in read order. One job owns writes;
// readers may take snapshots concurrently. The owner must Close the writer.
type KeptOutput struct {
	directory string
	head      *os.File
	segment   *os.File
	layout    *keptLayout
	closeOnce sync.Once
	closeErr  error
	requests  chan keptRequest
	stop      chan struct{}
	done      chan struct{}
}
type keptSegment struct {
	path   string
	length int64
	output uint64
}

// keptLayout is owned by the output worker and immutable after it stops.
type keptLayout struct {
	head      keptSegment
	segments  []keptSegment
	endLength int64
	leftOut   uint64
	next      uint64
}
type keptRequest struct {
	ctx      context.Context
	stream   runnerwire.OutputStream
	data     []byte
	write    chan error
	snapshot chan keptSnapshotReply
}
type keptSnapshotReply struct {
	reader io.ReadCloser
	err    error
}

// CreateKeptOutput creates the output files under directory.
func CreateKeptOutput(ctx context.Context, directory string) (*KeptOutput, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if err := os.Mkdir(directory, 0700); err != nil {
		return nil, err
	}
	path := filepath.Join(directory, "head")
	file, err := cmdsdk.Retry(ctx, func() (*os.File, error) { return os.Create(path) })
	if err != nil {
		return nil, err
	}
	output := &KeptOutput{directory: directory, head: file, layout: &keptLayout{head: keptSegment{path: path}}, requests: make(chan keptRequest), stop: make(chan struct{}), done: make(chan struct{})}
	go output.serve()
	return output, nil
}

// Write appends one stream read, preserving the first and newest records within
// the runner wire's kept-byte bound.
func (o *KeptOutput) Write(ctx context.Context, stream runnerwire.OutputStream, data []byte) error {
	reply := make(chan error, 1)
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-o.done:
		return os.ErrClosed
	case o.requests <- keptRequest{ctx: ctx, stream: stream, data: data, write: reply}:
	}
	// A queued write owns data until its reply, including cancellation.
	return <-reply
}

// writeOwned records one read while the output worker exclusively owns its layout.
func (o *KeptOutput) writeOwned(ctx context.Context, stream runnerwire.OutputStream, bytes []byte) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if o.head != nil {
		room := runnerwire.JobKeptPartBytes - o.layout.head.length
		taken := min(len(bytes), int(room))
		var record []byte
		for taken > 0 {
			var err error
			record, err = runnerwire.EncodeRecord(&runnerwire.KeptOutput{Stream: stream, Bytes: runnerwire.WireBytes(bytes[:taken])})
			if err != nil {
				return err
			}
			if len(record) <= int(room) {
				break
			}
			taken = max(0, taken-(len(record)-int(room)))
		}
		if taken > 0 {
			if _, err := o.head.Write(record); err != nil {
				return err
			}
			o.layout.head.length += int64(len(record))
		}
		if taken == len(bytes) {
			return nil
		}
		if err := o.head.Close(); err != nil {
			return err
		}
		o.head = nil
		bytes = bytes[taken:]
	}
	return o.writeEnd(stream, bytes)
}

// Reader reads kept output while the job runs and after it ends.
func (o *KeptOutput) Reader() *KeptReader {
	return &KeptReader{output: o}
}

// Close releases writer files without removing the output. It is idempotent.
func (o *KeptOutput) Close() error {
	o.closeOnce.Do(func() { close(o.stop) })
	<-o.done
	return o.closeErr
}

// serve serializes file mutation and snapshot opens without holding a mutex over IO.
func (o *KeptOutput) serve() {
	defer close(o.done)
	defer func() {
		if o.head != nil {
			o.closeErr = errors.Join(o.closeErr, o.head.Close())
		}
		if o.segment != nil {
			o.closeErr = errors.Join(o.closeErr, o.segment.Close())
		}
	}()
	for {
		select {
		case <-o.stop:
			return
		case request := <-o.requests:
			if request.write != nil {
				request.write <- o.writeOwned(request.ctx, request.stream, request.data)
			} else {
				reader, err := o.snapshot(request.ctx)
				request.snapshot <- keptSnapshotReply{reader: reader, err: err}
			}
		}
	}
}

// KeptReader takes snapshots of a job's kept output.
type KeptReader struct{ output *KeptOutput }

// Snapshot opens the retained files at their current lengths. The caller closes
// the returned reader on success, failure or cancellation, including a partial read.
// Records are head, any left-out count, then tail; later writes do not extend it.
func (r *KeptReader) Snapshot(ctx context.Context) (io.ReadCloser, error) {
	o := r.output
	reply := make(chan keptSnapshotReply, 1)
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-o.done:
		return o.snapshot(ctx)
	case o.requests <- keptRequest{ctx: ctx, snapshot: reply}:
	}
	// The worker opens every retained part before it may rotate another segment.
	result := <-reply
	return result.reader, result.err
}

// snapshot opens one immutable view, either on the worker or after it has stopped.
func (o *KeptOutput) snapshot(ctx context.Context) (io.ReadCloser, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	l := o.layout
	segments := append([]keptSegment{l.head}, l.segments...)
	leftOut := l.leftOut
	result := &keptSnapshot{}
	for index, segment := range segments {
		file, err := os.Open(segment.path)
		if err != nil {
			_ = result.Close()
			return nil, err
		} // Release all successfully opened parts.
		result.files = append(result.files, file)
		result.parts = append(result.parts, io.NewSectionReader(file, 0, segment.length))
		if index == 0 && leftOut > 0 {
			record, err := runnerwire.EncodeRecord(&runnerwire.KeptLeftOut{Bytes: leftOut})
			if err != nil {
				_ = result.Close()
				return nil, err
			}
			result.parts = append(result.parts, bytes.NewReader(record))
		}
	}
	result.reader = io.MultiReader(result.parts...)
	return result, nil
}

// writeEnd rotates the bounded tail of the job's retained reads.
func (o *KeptOutput) writeEnd(stream runnerwire.OutputStream, data []byte) error {
	l := o.layout
	full := len(l.segments) == 0 || l.segments[len(l.segments)-1].length >= 1024*1024
	next := l.next
	if full || o.segment == nil {
		if o.segment != nil {
			if err := o.segment.Close(); err != nil {
				return err
			}
		}
		path := filepath.Join(o.directory, fmt.Sprintf("end-%d", next))
		file, err := os.Create(path)
		if err != nil {
			return err
		}
		o.segment = file
		l.next++
		l.segments = append(l.segments, keptSegment{path: path})
	}
	record, err := runnerwire.EncodeRecord(&runnerwire.KeptOutput{Stream: stream, Bytes: runnerwire.WireBytes(data)})
	if err != nil {
		return err
	}
	if _, err = o.segment.Write(record); err != nil {
		return err
	}
	segment := &l.segments[len(l.segments)-1]
	segment.length += int64(len(record))
	segment.output += uint64(len(data))
	l.endLength += int64(len(record))
	var dropped []string
	for l.endLength > runnerwire.JobKeptPartBytes && len(l.segments) > 1 {
		oldest := l.segments[0]
		l.segments = l.segments[1:]
		l.endLength -= oldest.length
		l.leftOut += oldest.output
		dropped = append(dropped, oldest.path)
	}
	for _, path := range dropped {
		if err := os.Remove(path); err != nil {
			return err
		}
	}
	return nil
}

type keptSnapshot struct {
	reader io.Reader
	parts  []io.Reader
	files  []*os.File
	once   sync.Once
	err    error
}

func (s *keptSnapshot) Read(bytes []byte) (int, error) { return s.reader.Read(bytes) }
func (s *keptSnapshot) Close() error {
	s.once.Do(func() {
		for _, file := range s.files {
			s.err = errors.Join(s.err, file.Close())
		}
	})
	return s.err
}
