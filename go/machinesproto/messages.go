package machinesproto

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"bytes"
	"encoding/json/jsontext"
	"errors"

	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/runnerproto"
)

// MaxLineBytes is the longest line either end reads, its newline excluded. The
// largest real message is a few kilobytes.
const MaxLineBytes = 1 << 20

// An InvalidError says that a message breaks the contract. It names the field
// and the rule, never the value.
type InvalidError = wire.InvalidError

// A Request is what the backend sends: an id the client chooses, which the reply
// names, and the call, whose members (op and params) are the request's own. A
// member the request does not declare is ignored.
//
//demi:wire open
type Request struct {
	ID   string `json:"id" check:"chars=1.."`
	Call Call   `json:",inline"`
}

// A Call is an operation and its parameters. The variants are the operations of
// managed-hosts.md § Control and ownership. A device id or base version travels
// as a string: a name that breaks the image-name rule is the operation's error,
// not a malformed message, so the connection stays usable. A member of the
// parameters that the operation does not declare is ignored.
//
//demi:union tag=op content=params
type Call interface{ call() }

// ReconcileParams stops and saves every device, recovers incomplete operations
// and installs the network policy again.
//
//demi:variant reconcile open
type ReconcileParams struct{}

// CurrentBaseVersionParams reads the configured base.
//
//demi:variant current_base_version open
type CurrentBaseVersionParams struct{}

// ImageStateParams reads a device's committed generation.
//
//demi:variant image_state open
type ImageStateParams struct {
	DeviceID string `json:"deviceId" check:"chars=1.."`
}

// RuntimeStateParams reads whether the manager runs a sandbox for a device,
// after the device's earlier operations.
//
//demi:variant runtime_state open
type RuntimeStateParams struct {
	DeviceID string `json:"deviceId" check:"chars=1.."`
}

// WakeParams creates first-use storage or recovers existing storage, then starts
// one sandbox with the boot credential.
//
//demi:variant wake open
type WakeParams struct {
	DeviceID string                  `json:"deviceId" check:"chars=1.."`
	Boot     runnerproto.ManagedBoot `json:"boot" check:"func=runnerproto.Validate"`
}

// HibernateParams stops execution, saves storage and releases runtime
// resources.
//
//demi:variant hibernate open
type HibernateParams struct {
	DeviceID string `json:"deviceId" check:"chars=1.."`
}

// CheckpointParams publishes the running device's storage while preserving its
// processes.
//
//demi:variant checkpoint open
type CheckpointParams struct {
	DeviceID string `json:"deviceId" check:"chars=1.."`
}

// GrowVolumeParams grows one of the running device's filesystems to at least
// Bytes.
//
//demi:variant grow_volume open
type GrowVolumeParams struct {
	DeviceID string `json:"deviceId" check:"chars=1.."`
	Volume   Volume `json:"volume" check:"oneof=system|home"`
	Bytes    uint64 `json:"bytes" check:"range=1.."`
}

// ResetParams publishes a clean system on BaseVersion with the retained home,
// once per OperationID; it does not boot.
//
//demi:variant reset open
type ResetParams struct {
	DeviceID    string `json:"deviceId" check:"chars=1.."`
	OperationID string `json:"operationId" check:"chars=1.."`
	BaseVersion string `json:"baseVersion" check:"chars=1.."`
}

func (ReconcileParams) call()          {}
func (CurrentBaseVersionParams) call() {}
func (ImageStateParams) call()         {}
func (RuntimeStateParams) call()       {}
func (WakeParams) call()               {}
func (HibernateParams) call()          {}
func (CheckpointParams) call()         {}
func (GrowVolumeParams) call()         {}
func (ResetParams) call()              {}

// OperationName returns the name of call's operation on the wire, such as
// "grow_volume", for a log line.
func OperationName(call Call) string {
	switch call.(type) {
	case ReconcileParams:
		return "reconcile"
	case CurrentBaseVersionParams:
		return "current_base_version"
	case ImageStateParams:
		return "image_state"
	case RuntimeStateParams:
		return "runtime_state"
	case WakeParams:
		return "wake"
	case HibernateParams:
		return "hibernate"
	case CheckpointParams:
		return "checkpoint"
	case GrowVolumeParams:
		return "grow_volume"
	case ResetParams:
		return "reset"
	}
	return "unknown"
}

// A Response is what the manager sends: the reply to a request, or the death of
// a device's sandbox, which every connection receives. A member a response does
// not declare is ignored.
//
//demi:union tag=type
type Response interface{ response() }

// OK is the reply of an operation that succeeded. Result is the operation's
// result as JSON: null, a base version, an [ImageState] or null, a
// [RuntimeState]. It is always written.
//
//demi:variant ok open
type OK struct {
	ID     string         `json:"id" check:"chars=1.."`
	Result jsontext.Value `json:"result"`
}

// Failure is the reply of an operation that failed.
//
//demi:variant error open
type Failure struct {
	ID      string `json:"id" check:"chars=1.."`
	Message string `json:"message"`
}

// Death says a device's sandbox exited without being asked to stop.
//
//demi:variant death open
type Death struct {
	DeviceID string `json:"deviceId" check:"chars=1.."`
}

func (OK) response()      {}
func (Failure) response() {}
func (Death) response()   {}

// DecodeRequest decodes one request line, its newline removed: a JSON object
// with a non-empty id, an op and its params.
func DecodeRequest(line []byte) (Request, error) {
	return decode[Request](line)
}

// DecodeResponse decodes one response line, its newline removed.
func DecodeResponse(line []byte) (Response, error) {
	return decode[Response](line)
}

// EncodeRequest returns the line that carries request: compact JSON and a
// newline.
func EncodeRequest(request Request) ([]byte, error) {
	if request.Call == nil {
		return nil, errors.New("a request needs a call")
	}
	return encodeLine(request)
}

// EncodeResponse returns the line that carries response: compact JSON and a
// newline.
func EncodeResponse(response Response) ([]byte, error) {
	return encodeLine(response)
}

// encodeLine is the compact JSON of a message and a newline.
func encodeLine[T any](message T) ([]byte, error) {
	data, err := encode(message)
	if err != nil {
		return nil, err
	}
	return append(bytes.Clone(data), '\n'), nil
}
