package hostaccess

//revive:disable:unused-parameter

import (
	"context"
	"net/http"
	"sync"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// Lease is the edge's hold on admitted work. The receiver defers Release.
// Revocation cancels Context and releases shard admission without waiting for
// the edge; Release is idempotent and does not wait.
type Lease struct{}

// Released observes the acquiring edge's explicit release.
type Released struct{}

// NewLease makes an edge lease ended by ctx and its release notification.
func NewLease(ctx context.Context) (*Lease, *Released) { panic("not written: b-hostaccess") }

// Context ends when the lease is revoked or released; the edge stops copying.
func (l *Lease) Context() context.Context { panic("not written: b-hostaccess") }

// Release ends this edge's hold exactly once.
func (l *Lease) Release() { panic("not written: b-hostaccess") }

// Wait waits for the edge to release, or returns ctx.Err().
func (r *Released) Wait(ctx context.Context) error { panic("not written: b-hostaccess") }

// TransferSet atomically checks admission and registers transfers under the
// shard mutex, including those waiting for Host access. Do not copy it.
type TransferSet struct{}

// NewTransferSet makes a registry using its owning shard's mutex.
func NewTransferSet(mu *sync.Mutex) *TransferSet { panic("not written: b-hostaccess") }

// AnyOpen reports whether a transfer or user stream is registered.
func (s *TransferSet) AnyOpen() bool { panic("not written: b-hostaccess") }

// Open registers before waiting for admission, or returns TransfersBusy.
func (s *TransferSet) Open() (*OpenTransfer, error) { panic("not written: b-hostaccess") }

// Close atomically closes admission, revokes registered work and awaits its
// admission release outside the mutex. Concurrent closes each retain a hold;
// only releasing the last hold reopens admission. Cancellation releases this
// call's closing hold, never another caller's hold.
func (s *TransferSet) Close(ctx context.Context) (*TransfersClosed, error) {
	panic("not written: b-hostaccess")
}

// TransfersBusy means a transition closed transfer and stream admission.
type TransfersBusy struct{}

// Error returns the transfer admission refusal.
func (e *TransfersBusy) Error() string { panic("not written: b-hostaccess") }

// OpenTransfer owns one registered transfer, including its admission wait.
type OpenTransfer struct{}

// Context is canceled when a transition ends the registered work.
func (o *OpenTransfer) Context() context.Context { panic("not written: b-hostaccess") }

// Release unregisters the work once its admission has been released.
func (o *OpenTransfer) Release() { panic("not written: b-hostaccess") }

// TransfersClosed keeps admission closed until explicitly released.
type TransfersClosed struct{}

// Release ends this closing hold exactly once, without waiting.
func (c *TransfersClosed) Release() { panic("not written: b-hostaccess") }

// DownloadRequest describes the requested file, version, head and range.
type DownloadRequest struct {
	Path string
	// Version is the version the page expects the file to still have.
	Version *string
	// Head asks for only the answer's head, as HEAD asks.
	Head        bool
	Range       *string
	IfNoneMatch *string
}

// Download is the answer decided from a file's metadata.
//
//sumtype:decl
type Download interface{ download() }

// DownloadNotAFile means nothing at the path is a regular file.
type DownloadNotAFile struct{}

func (*DownloadNotAFile) download() { panic("not written: b-hostaccess") }

// DownloadChanged means the file is no longer the requested version.
type DownloadChanged struct{}

func (*DownloadChanged) download() { panic("not written: b-hostaccess") }

// DownloadNotModified means the condition names the file's version.
type DownloadNotModified struct{ Version string }

func (*DownloadNotModified) download() { panic("not written: b-hostaccess") }

// DownloadHead carries metadata alone for HEAD, an empty or refused range.
type DownloadHead struct {
	Stat host.FileStat
	Part RangeAnswer
}

func (*DownloadHead) download() { panic("not written: b-hostaccess") }

// DownloadStream carries the requested bytes; the edge defers Lease.Release.
type DownloadStream struct {
	Stat  host.FileStat
	Part  RangeAnswer
	Body  *remotehost.PipeReader
	Lease *Lease
}

func (*DownloadStream) download() { panic("not written: b-hostaccess") }

// Upload is the answer before upload bytes move.
//
//sumtype:decl
type Upload interface{ upload() }

// UploadIsDirectory refuses to overwrite a directory.
type UploadIsDirectory struct{}

func (*UploadIsDirectory) upload() { panic("not written: b-hostaccess") }

// UploadExists refuses to overwrite a file without replace.
type UploadExists struct{}

func (*UploadExists) upload() { panic("not written: b-hostaccess") }

// OpenUpload accepts bytes and installs the file whole or not at all. The edge
// closes Writer after its final byte, waits for Written, and defers Release.
type OpenUpload struct {
	Writer *remotehost.PipeWriter
	Lease  *Lease
}

func (*OpenUpload) upload() { panic("not written: b-hostaccess") }

// Written waits for the Host's write outcome; cancellation ends this wait.
func (u *OpenUpload) Written(ctx context.Context) error { panic("not written: b-hostaccess") }

// OpenDownload admits a main-Host download and returns bytes or metadata.
func OpenDownload(ctx context.Context, shard HostShard, id webapi.ConversationID, request DownloadRequest) (Download, error) {
	panic("not written: b-hostaccess")
}

// UploadFile admits an upload to the main Host, replacing files only if asked.
// The returned OpenUpload transfers explicit lease ownership to the edge.
func UploadFile(ctx context.Context, shard HostShard, id webapi.ConversationID, path string, replace bool) (Upload, error) {
	panic("not written: b-hostaccess")
}

// FileVersion returns the weak size-and-millisecond modification-time ETag.
func FileVersion(stat host.FileStat) string { panic("not written: b-hostaccess") }

// RangeAnswer is an immutable whole, partial or unsatisfiable file range.
// Construct it with RangeOf.
type RangeAnswer struct{}

// RangeOf interprets a single byte range, including suffix and overflowing
// bounds. Invalid, multiple or reversed ranges request the whole file.
func RangeOf(header *string, size uint64) RangeAnswer { panic("not written: b-hostaccess") }

// Status returns 200, 206 or 416 for this answer.
func (r RangeAnswer) Status() int { panic("not written: b-hostaccess") }

// Range returns the bytes to send, or nil for an unsatisfiable range.
func (r RangeAnswer) Range() *host.ByteRange { panic("not written: b-hostaccess") }

// PartOf returns the selected part of the complete file bytes, or nil on refusal.
func (r RangeAnswer) PartOf(data []byte) []byte { panic("not written: b-hostaccess") }

// Headers describes the answer's part. The edge omits length for streamed bodies.
func (r RangeAnswer) Headers() http.Header { panic("not written: b-hostaccess") }
