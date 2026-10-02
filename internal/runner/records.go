package runner

import (
	"errors"

	"github.com/wspl/demi/internal/runnerwire"
)

//go:generate go run ../../tools/contractgen

// +demi:root
// +demi:strict
type runnerConfig struct {
	BackendURL runnerwire.BackendURL `json:"backendUrl"`
	DeviceID   *deviceID             `json:"deviceId,omitempty"`
}

// The id the backend gave the device; never empty.
// +demi:check validateDeviceID
type deviceID string

func validateDeviceID(id deviceID) error {
	if id == "" {
		return errors.New("empty device ID in runner config")
	}
	return nil
}

// The runner that holds the installation, for `status` and `drain`: its
// local endpoint, the secret its management requests carry, and its release.
// +demi:root
// +demi:strict
// +demi:check validateActiveRunner
type activeRunner struct {
	Endpoint string `json:"endpoint"`
	Secret   string `json:"secret"`
	Release  string `json:"release"`
}

func validateActiveRunner(v activeRunner) error {
	valid := len(v.Secret) == 32
	for _, b := range []byte(v.Secret) {
		if (b < '0' || b > '9') && (b < 'a' || b > 'f') {
			valid = false
		}
	}
	if v.Endpoint == "" || v.Release == "" || !valid {
		return errors.New("invalid active runner record")
	}
	return nil
}

// +demi:enum connecting claim_pending online rejected
type phase string

const (
	connecting   phase = "connecting"
	claimPending phase = "claim_pending"
	online       phase = "online"
	rejected     phase = "rejected"
)

// +demi:enum status drain
type action string

const (
	statusAction action = "status"
	drainAction  action = "drain"
)

// +demi:root
// +demi:strict
type managementRequest struct {
	Secret string `json:"secret"`
	Action action `json:"action"`
}

// +demi:root
type managementStatus struct {
	Release  string `json:"release"`
	Phase    phase  `json:"phase"`
	Draining bool   `json:"draining"`
	Jobs     uint64 `json:"jobs"`
}

// One line as the files keep it, a JSON object per line. `seq` is the
// cursor: it grows by one per line across both files and across restarts.
// +demi:root
// +demi:strict
type logLine struct {
	Seq uint64 `json:"seq"`
	// Milliseconds since the Unix epoch.
	At             int64   `json:"at"`
	Source         string  `json:"source"`
	ConversationID *string `json:"conversationId,omitempty"`
	Text           string  `json:"text"`
}
