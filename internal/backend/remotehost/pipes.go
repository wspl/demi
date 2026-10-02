package remotehost

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// Arrival is the time allowed for both pipe ends to arrive.
const Arrival = 120 * time.Second

// PipeFailure is why a pipe failed; Message begins "pipe failed: ".
type PipeFailure struct{ Message string }

// Error describes the pipe failure.
func (e *PipeFailure) Error() string { panic("not written: b-remotehost") }

// PipeRefusal identifies why a device cannot claim an end.
type PipeRefusal uint8

const (
	// PipeNotFound means no such pipe, an ended pipe, or another device's end.
	PipeNotFound PipeRefusal = iota
	// PipeAlreadyConnected means the end has already been claimed.
	PipeAlreadyConnected
)

// Error describes why the device cannot claim the end.
func (e PipeRefusal) Error() string { panic("not written: b-remotehost") }

// PipeErrorKind identifies why the backend cannot take an end.
type PipeErrorKind uint8

const (
	// PipeSettled means the pipe is over.
	PipeSettled PipeErrorKind = iota
	// PipeAlreadyFixed means the end's assignment is fixed.
	PipeAlreadyFixed
	// PipeTaken means the end has already been taken.
	PipeTaken
)

// PipeError describes an unavailable backend end (source or sink).
type PipeError struct {
	Kind PipeErrorKind
	End  string
}

// Error describes why the backend cannot take the end.
func (e *PipeError) Error() string { panic("not written: b-remotehost") }

// Pipes owns pipe rendezvous records and their expiry workers.
// Its owner must Close it; pipe ends may be handed to other goroutines.
type Pipes struct{ _ byte }

// NewPipes creates a broker with the supplied arrival timeout.
func NewPipes(arrival time.Duration) *Pipes { panic("not written: b-remotehost") }

// Mint creates a pipe; nil endpoints are initially unassigned.
func (p *Pipes) Mint(source, sink *string) *Pipe { panic("not written: b-remotehost") }

// FromDevice creates a pipe sourced by device.
func (p *Pipes) FromDevice(device string) *Pipe { panic("not written: b-remotehost") }

// ToDevice creates a pipe drained by device.
func (p *Pipes) ToDevice(device string) *Pipe { panic("not written: b-remotehost") }

// Pipe looks up a live pipe by ID.
func (p *Pipes) Pipe(id string) (*Pipe, bool) { panic("not written: b-remotehost") }

// ClaimSource takes the source end assigned to device.
func (p *Pipes) ClaimSource(id, device string) (*DeviceSource, error) {
	panic("not written: b-remotehost")
}

// ClaimSink takes the sink end assigned to device.
func (p *Pipes) ClaimSink(id, device string) (*DeviceSink, error) { panic("not written: b-remotehost") }

// Fail fails the named pipe for both ends.
func (p *Pipes) Fail(id, reason string) { panic("not written: b-remotehost") }

// FailFromDevice fails a pipe only if device owns one of its ends.
func (p *Pipes) FailFromDevice(id, device, reason string) bool { panic("not written: b-remotehost") }

// DeviceGone fails all ends belonging to device.
func (p *Pipes) DeviceGone(device string) { panic("not written: b-remotehost") }

// Close fails remaining pipes and joins expiry workers. It is idempotent.
func (p *Pipes) Close(ctx context.Context) error { panic("not written: b-remotehost") }

// Pipe is a handle to a broker record, independent of ownership of its ends.
type Pipe struct{ _ byte }

// ID returns the pipe's identifier.
func (p *Pipe) ID() string { panic("not written: b-remotehost") }

// WireRef returns the pipe reference sent to the runner.
func (p *Pipe) WireRef() runnerwire.PipeRef { panic("not written: b-remotehost") }

// Reader takes the backend sink. The caller must close it or consume it to EOF.
func (p *Pipe) Reader() (*PipeReader, error) { panic("not written: b-remotehost") }

// Writer takes the backend source. The caller must End or Fail it.
func (p *Pipe) Writer() (*PipeWriter, error) { panic("not written: b-remotehost") }

// HoldSource reserves an unassigned source for later attachment.
func (p *Pipe) HoldSource() error { panic("not written: b-remotehost") }

// SinkTo assigns the sink to device.
func (p *Pipe) SinkTo(device string) error { panic("not written: b-remotehost") }

// SourceFrom assigns the source to device.
func (p *Pipe) SourceFrom(device string) error { panic("not written: b-remotehost") }

// Fail fails both ends with reason.
func (p *Pipe) Fail(reason string) { panic("not written: b-remotehost") }

// Done waits for successful draining or failure.
func (p *Pipe) Done(ctx context.Context) error { panic("not written: b-remotehost") }

// Failure returns the current failure, or nil while open or successfully drained.
func (p *Pipe) Failure() *PipeFailure { panic("not written: b-remotehost") }

// Failed waits for failure; successful draining does not end this wait.
func (p *Pipe) Failed(ctx context.Context) (*PipeFailure, error) { panic("not written: b-remotehost") }

// PipeReader is the backend's owned reading end. Do not read it concurrently.
type PipeReader struct{ _ byte }

// Next returns the next byte chunk, or io.EOF once drained.
func (r *PipeReader) Next(ctx context.Context) ([]byte, error) { panic("not written: b-remotehost") }

// Read reads bytes with cancellation and implements host.ByteStream.
func (r *PipeReader) Read(ctx context.Context, bytes []byte) (int, error) {
	panic("not written: b-remotehost")
}

// Fail abandons the reader and fails both ends.
func (r *PipeReader) Fail(reason string) { panic("not written: b-remotehost") }

// Close releases the backend reader and marks an early read as drained. It is idempotent.
func (r *PipeReader) Close(ctx context.Context) error { panic("not written: b-remotehost") }

// PipeWriter is the backend's owned writing end. Do not write it concurrently.
// End or Fail releases it; losing a Go reference does not close a pipe.
type PipeWriter struct{ _ byte }

// Write waits until the reader has room for bytes.
func (w *PipeWriter) Write(ctx context.Context, bytes []byte) error {
	panic("not written: b-remotehost")
}

// End ends input cleanly and releases the writer.
func (w *PipeWriter) End() { panic("not written: b-remotehost") }

// Fail fails both ends and releases the writer.
func (w *PipeWriter) Fail(reason string) { panic("not written: b-remotehost") }

// DeviceSource is a claimed device source; Pump or Fail must release it.
type DeviceSource struct{ _ byte }

// Pump moves the body into the pipe, closes the body and releases the source on every path.
func (s *DeviceSource) Pump(ctx context.Context, body host.ByteStream) error {
	panic("not written: b-remotehost")
}

// Fail releases an abandoned claim and fails the pipe.
func (s *DeviceSource) Fail(reason string) { panic("not written: b-remotehost") }

// DeviceSink is a claimed device sink; its owner closes it or drains it to EOF.
type DeviceSink struct{ _ byte }

// SourceArrived waits until the source is present or the pipe fails.
func (s *DeviceSink) SourceArrived(ctx context.Context) error { panic("not written: b-remotehost") }

// Next returns the next byte chunk, or io.EOF once drained.
func (s *DeviceSink) Next(ctx context.Context) ([]byte, error) { panic("not written: b-remotehost") }

// Read reads bytes with cancellation.
func (s *DeviceSink) Read(ctx context.Context, bytes []byte) (int, error) {
	panic("not written: b-remotehost")
}

// Close releases the sink, failing an incompletely drained pipe. It is idempotent.
func (s *DeviceSink) Close(ctx context.Context) error { panic("not written: b-remotehost") }

var _ host.ByteStream = (*PipeReader)(nil)
var _ host.ByteStream = (*DeviceSink)(nil)
