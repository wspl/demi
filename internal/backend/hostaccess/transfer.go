package hostaccess

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"sync"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// Lease is the edge's hold on admitted work. The receiver defers Release.
// Revocation cancels Context and releases shard admission without waiting for
// the edge; Release is idempotent and does not wait.
type Lease struct {
	ctx    context.Context
	cancel context.CancelFunc
	once   sync.Once
}

// NewLease makes an edge lease that ctx or Release ends.
func NewLease(ctx context.Context) *Lease {
	lifetime, cancel := context.WithCancel(ctx)
	return &Lease{ctx: lifetime, cancel: cancel}
}

// Context ends when the lease is revoked or released; the edge stops copying.
func (l *Lease) Context() context.Context {
	return l.ctx
}

// Release ends this edge's hold exactly once.
func (l *Lease) Release() {
	l.once.Do(func() {
		l.cancel()
	})
}

// TransferSet atomically checks admission and registers transfers under the
// shard mutex, including those waiting for Host access. Do not copy it.
type TransferSet struct {
	mu       *sync.Mutex // Owning shard mutex; protects closing holds and registrations.
	closings int
	open     map[*OpenTransfer]struct{}
}

// NewTransferSet makes a registry using its owning shard's mutex.
func NewTransferSet(mu *sync.Mutex) *TransferSet {
	return &TransferSet{mu: mu, open: make(map[*OpenTransfer]struct{})}
}

// AnyOpen reports whether a transfer or user stream is registered.
func (s *TransferSet) AnyOpen() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return len(s.open) != 0
}

// Open registers before waiting for admission, or returns Busy.
func (s *TransferSet) Open() (*OpenTransfer, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closings != 0 {
		return nil, Busy
	}
	ctx, cancel := context.WithCancel(context.Background())
	open := &OpenTransfer{set: s, ctx: ctx, cancel: cancel, done: make(chan struct{})}
	s.open[open] = struct{}{}
	return open, nil
}

// Close atomically closes admission, revokes registered work and awaits its
// admission release outside the mutex. Concurrent closes each retain a hold;
// only releasing the last hold reopens admission. Cancellation releases this
// call's closing hold, never another caller's hold.
func (s *TransferSet) Close(ctx context.Context) (*TransfersClosed, error) {
	s.mu.Lock()
	s.closings++
	open := make([]*OpenTransfer, 0, len(s.open))
	for transfer := range s.open {
		open = append(open, transfer)
	}
	s.mu.Unlock()
	closed := &TransfersClosed{set: s}
	for _, transfer := range open {
		transfer.cancel()
	}
	for _, transfer := range open {
		select {
		case <-transfer.done:
		case <-ctx.Done():
			closed.Release()
			return nil, ctx.Err()
		}
	}
	return closed, nil
}

// OpenTransfer owns one registered transfer, including its admission wait.
type OpenTransfer struct {
	set    *TransferSet
	ctx    context.Context
	cancel context.CancelFunc
	once   sync.Once
	done   chan struct{}
}

// Context is canceled when a transition ends the registered work.
func (o *OpenTransfer) Context() context.Context {
	return o.ctx
}

// Release unregisters the work once its admission has been released.
func (o *OpenTransfer) Release() {
	o.once.Do(func() {
		o.cancel()
		o.set.mu.Lock()
		delete(o.set.open, o)
		o.set.mu.Unlock()
		close(o.done)
	})
}

// TransfersClosed keeps admission closed until explicitly released.
type TransfersClosed struct {
	set  *TransferSet
	once sync.Once
}

// Release ends this closing hold exactly once, without waiting.
func (c *TransfersClosed) Release() {
	c.once.Do(func() {
		c.set.mu.Lock()
		c.set.closings--
		c.set.mu.Unlock()
	})
}

// DownloadRequest describes the requested file, version, head and range.
type DownloadRequest struct {
	Path string
	// Version is the version the page expects the file to still have.
	Version *string
	// Head asks for only the answer's head, as HEAD asks.
	Head        bool
	Range       string
	IfNoneMatch string
}

// Download is the answer decided from a file's metadata.
//
//sumtype:decl
type Download interface{ download() }

// ErrNotAFile means nothing at the path is a regular file.
var ErrNotAFile = errors.New("not a regular file")

// ErrFileChanged means the file is no longer the version the request expects.
var ErrFileChanged = errors.New("the file is no longer the version asked for")

// DownloadNotModified means the condition names the file's version.
type DownloadNotModified struct{ Version string }

func (*DownloadNotModified) download() {}

// DownloadHead carries metadata alone for HEAD, an empty or refused range.
type DownloadHead struct {
	Stat host.FileStat
	Part RangeAnswer
}

func (*DownloadHead) download() {}

// DownloadStream carries the requested bytes; the edge defers Lease.Release.
type DownloadStream struct {
	Stat  host.FileStat
	Part  RangeAnswer
	Body  *remotehost.PipeReader
	Lease *Lease
}

func (*DownloadStream) download() {}

// ErrIsDirectory refuses an upload that would overwrite a directory.
var ErrIsDirectory = errors.New("a directory is at this path")

// ErrFileExists refuses an upload that would overwrite a file without replace.
var ErrFileExists = errors.New("a file is already at this path")

// OpenUpload accepts bytes and installs the file whole or not at all. The edge
// closes Writer after its final byte, waits for Written, and defers Release.
type OpenUpload struct {
	written chan struct{}
	outcome error
	Writer  *remotehost.PipeWriter
	Lease   *Lease
}

// Written waits for the Host's write outcome; cancellation ends this wait.
func (u *OpenUpload) Written(ctx context.Context) error {
	select {
	case <-u.written:
		return u.outcome
	case <-ctx.Done():
		return ctx.Err()
	}
}

// OpenDownload admits a main-Host download and returns bytes or metadata.
func OpenDownload(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	request DownloadRequest,
) (Download, error) {
	open, waitCtx, stop, err := registerTransfer(ctx, shard, id)
	if err != nil {
		return nil, err
	}
	defer stop()
	handed := false
	defer func() {
		if !handed {
			open.Release()
		}
	}()
	admitted, err := AdmitHost(waitCtx, shard, id, nil)
	if err != nil {
		return nil, transferError(open, err)
	}
	defer func() {
		if !handed {
			admitted.Release()
		}
	}()
	stat, err := admitted.Host.Host.FS().Stat(waitCtx, request.Path)
	if err != nil {
		return nil, transferError(open, accessError(err))
	}
	if stat.Kind != host.File {
		return nil, ErrNotAFile
	}
	version := FileVersion(stat)
	if request.Version != nil && *request.Version != version {
		return nil, ErrFileChanged
	}
	if notModified(request.IfNoneMatch, version) {
		return &DownloadNotModified{Version: version}, nil
	}
	part := RangeOf(request.Range, stat.Size)
	span, ok := part.Range()
	if request.Head || !ok || *span.Length == 0 {
		return &DownloadHead{Stat: stat, Part: part}, nil
	}
	reader, err := admitted.Host.Host.ReadPipe(waitCtx, request.Path, span)
	if err != nil {
		return nil, transferError(open, accessError(err))
	}
	lease := NewLease(open.Context())
	// AdmitHost's owner registration moves to this worker and ends on release.
	go func() {
		<-lease.Context().Done()
		// The edge owns and closes its reader when this lease is cancelled.
		lease.Release()
		admitted.Release()
		open.Release()
	}()
	handed = true
	return &DownloadStream{Stat: stat, Part: part, Body: reader, Lease: lease}, nil
}

// UploadFile admits an upload to the main Host, replacing files only if asked.
// The returned OpenUpload transfers explicit lease ownership to the edge.
func UploadFile(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	path string,
	replace bool,
) (*OpenUpload, error) {
	open, waitCtx, stop, err := registerTransfer(ctx, shard, id)
	if err != nil {
		return nil, err
	}
	defer stop()
	handed := false
	defer func() {
		if !handed {
			open.Release()
		}
	}()
	admitted, err := AdmitHost(waitCtx, shard, id, nil)
	if err != nil {
		return nil, transferError(open, err)
	}
	defer func() {
		if !handed {
			admitted.Release()
		}
	}()
	stat, err := admitted.Host.Host.FS().Stat(waitCtx, path)
	if err != nil && !hostCode(err, "ENOENT") {
		return nil, transferError(open, accessError(err))
	}
	if err == nil {
		if stat.Kind == host.Directory {
			return nil, ErrIsDirectory
		}
		if !replace {
			return nil, ErrFileExists
		}
	}
	pipe, err := admitted.Host.Host.WritePipe()
	if err != nil {
		return nil, accessError(err)
	}
	writer, err := pipe.Writer()
	if err != nil {
		pipe.Fail(err.Error())
		return nil, accessError(err)
	}
	result := writeUpload(admitted, open, path, pipe, writer)

	handed = true
	return result, nil
}

// FileVersion returns the weak size-and-millisecond modification-time ETag.
func FileVersion(stat host.FileStat) string {
	modified, _ := stat.Modified.Millisecond() // Host contracts validate timestamps on arrival.
	return fmt.Sprintf("W/\"%x-%s\"", stat.Size, strconv.FormatInt(modified, 16))
}

// RangeAnswer is an immutable whole, partial or unsatisfiable file range.
// Construct it with RangeOf.
type RangeAnswer struct {
	status              int
	start, length, size uint64
}

// RangeOf interprets a single byte range, including suffix and overflowing
// bounds. Invalid, multiple or reversed ranges request the whole file.
func RangeOf(header string, size uint64) RangeAnswer {
	whole := RangeAnswer{status: 200, size: size, length: size}
	text, ok := strings.CutPrefix(strings.TrimSpace(header), "bytes=")
	if !ok {
		return whole
	}
	first, last, ok := strings.Cut(text, "-")
	if !ok || first == "" && last == "" || strings.Trim(first+last, "0123456789") != "" {
		return whole
	}
	refused := RangeAnswer{status: 416, size: size}
	var start, end uint64
	if first == "" {
		suffix, err := strconv.ParseUint(last, 10, 64)
		if err != nil {
			suffix = ^uint64(0)
		}
		if suffix == 0 || size == 0 {
			return refused
		}
		if suffix < size {
			start = size - suffix
		}
		end = size - 1
		return RangeAnswer{status: 206, start: start, length: end - start + 1, size: size}
	}
	var err error
	start, err = strconv.ParseUint(first, 10, 64)
	if err != nil {
		return refused
	}
	end = ^uint64(0)
	if last != "" {
		parsed, err := strconv.ParseUint(last, 10, 64)
		if err == nil {
			end = parsed
		}
		if end < start {
			return whole
		}
	}
	if size == 0 || start >= size {
		return refused
	}
	end = min(end, size-1)
	return RangeAnswer{status: 206, start: start, length: end - start + 1, size: size}
}

// Status returns 200, 206 or 416 for this answer.
func (r RangeAnswer) Status() int {
	return r.status
}

// Range returns the bytes to send, and false for an unsatisfiable range.
func (r RangeAnswer) Range() (host.ByteRange, bool) {
	if r.status == 416 {
		return host.ByteRange{}, false
	}
	return host.ByteRange{Offset: r.start, Length: new(r.length)}, true
}

// PartOf returns the selected part of the complete file bytes, or nil on refusal.
func (r RangeAnswer) PartOf(data []byte) []byte {
	if r.status == 416 {
		return nil
	}
	return data[r.start : r.start+r.length]
}

// Headers describes the answer's part. The edge omits length for streamed bodies.
func (r RangeAnswer) Headers() http.Header {
	headers := make(http.Header)
	headers.Set("Accept-Ranges", "bytes")
	switch r.status {
	case 200:
		headers.Set("Content-Length", strconv.FormatUint(r.size, 10))
	case 206:
		headers.Set("Content-Length", strconv.FormatUint(r.length, 10))
		headers.Set("Content-Range", fmt.Sprintf("bytes %d-%d/%d", r.start, r.start+r.length-1, r.size))
	case 416:
		headers.Set("Content-Range", fmt.Sprintf("bytes */%d", r.size))
	}
	return headers
}

// registerTransfer makes a transition see an operation even while it awaits admission.
func registerTransfer(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
) (*OpenTransfer, context.Context, func(), error) {
	record, err := OwnedConversation(ctx, shard, id)
	if err != nil {
		return nil, nil, nil, err
	}
	open, err := shard.Conversations().Slot(record.ID).transfers.Open()
	if err != nil {
		return nil, nil, nil, &Error{Kind: AccessRefused, Cause: err}
	}
	waitCtx, cancel := context.WithCancel(ctx)
	cancelled := make(chan struct{})
	stop := context.AfterFunc(open.Context(), func() {
		defer close(cancelled)
		cancel()
	})
	return open, waitCtx, func() {
		if !stop() {
			<-cancelled
		}
		cancel()
	}, nil
}

// transferError gives a closing transition priority over an interrupted wait.
func transferError(open *OpenTransfer, err error) error {
	if open.Context().Err() != nil {
		return &Error{Kind: AccessRefused, Cause: Busy}
	}
	return err
}

// notModified compares file validators weakly, as If-None-Match requires.
func notModified(condition, version string) bool {
	if strings.TrimSpace(condition) == "*" {
		return true
	}
	for _, tag := range strings.Split(condition, ",") {
		tag = strings.TrimSpace(tag)
		for strings.HasPrefix(tag, "W/") {
			tag = strings.TrimPrefix(tag, "W/")
		}
		expected := version
		for strings.HasPrefix(expected, "W/") {
			expected = strings.TrimPrefix(expected, "W/")
		}
		if tag == expected {
			return true
		}
	}
	return false
}

// hostCode recognizes filesystem errno without comparing error text.
func hostCode(err error, code string) bool {
	var failure *host.Error
	return errors.As(err, &failure) && failure.Code == code
}

// writeUpload forwards cancellation only while the write is running.
func writeUpload(
	admitted *Admitted,
	open *OpenTransfer,
	path string,
	pipe *remotehost.Pipe,
	writer *remotehost.PipeWriter,
) *OpenUpload {
	// Normal write completion unregisters the transfer without revoking the
	// edge's lease. Forward cancellation only while the write is running.
	lease := NewLease(context.Background())
	revoked := make(chan struct{})
	stopRevocation := context.AfterFunc(open.Context(), func() {
		defer close(revoked)
		lease.Release()
	})
	result := &OpenUpload{Writer: writer, Lease: lease, written: make(chan struct{})}
	go func() {
		result.outcome = admitted.Host.Host.WriteFrom(lease.Context(), path, pipe, host.WriteOptions{})
		if !stopRevocation() {
			<-revoked
		}
		if result.outcome != nil {
			pipe.Fail("the upload ended before its last byte")
		}
		close(result.written)
		admitted.Release()
		open.Release()
	}()
	return result
}
