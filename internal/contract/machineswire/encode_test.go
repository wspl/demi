package machineswire

import (
	"testing"

	"github.com/wspl/demi/internal/contract/zodrt"
)

func TestEncodersRefuseInvalidValues(t *testing.T) {
	if _, err := EncodeMachineRequestJSON(GrowVolume{ID: "1", Params: GrowVolumeParams{DeviceID: "d", Volume: GrowVolumeParamsVolumeHome, Bytes: 0}}); err == nil {
		t.Error("a zero-byte volume encoded")
	}
	if _, err := EncodeMachineResponseJSON(ResponseDeath{DeviceID: ""}); err == nil {
		t.Error("a death without a device encoded")
	}
	if _, err := ImageStateResultValue(zodrt.NotNull(MachineImageState{Generation: "g 1", BaseVersion: "b", SystemBytes: 1, HomeBytes: 1})); err == nil {
		t.Error("an image state with an invalid generation encoded")
	}
}

// A reply's result is validated by the op's schema once the client knows the op.
func TestTypedResults(t *testing.T) {
	state, err := ParseImageStateResult(nil)
	if err != nil || state.Valid {
		t.Fatalf("null image state: %v %v", state, err)
	}
	if _, err := ParseRuntimeStateResult("paused"); err == nil {
		t.Error("an unknown runtime state parsed")
	}
	if _, err := ParseReconcileResult(map[string]any{}); err == nil {
		t.Error("a non-null reconcile result parsed")
	}
}
