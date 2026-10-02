package machinewire

import (
	"encoding/json"

	"github.com/wspl/demi/internal/contract"
)

// A call's parameters, and the result its `ok` reply carries.
type Operation[T any] interface {
	Call() MachineCall
	DecodeOutput([]byte) (T, error)
}

// Unit represents the absence of an operation result.
type Unit struct{}

// +demi:root
// +demi:union tag=op content=result
//
//sumtype:decl
type operationResult interface{ operationResult() }

// decodeOperationResult selects the result schema from the outstanding operation.
func decodeOperationResult[T any](op string, data []byte, decode func([]byte) (T, error)) (T, error) {
	envelope, err := contract.EncodeObject([]contract.Field{{Name: "op", Value: op}, {Name: "result", Value: json.RawMessage(data)}})
	if err != nil {
		var zero T
		return zero, err
	}
	return decode(envelope)
}

// +demi:variant operationResult reconcile
type reconcileResult struct {
}

// Call identifies the operation and carries its parameters.
func (p ReconcileParams) Call() MachineCall { return &Reconcile{Params: p} }

// DecodeOutput checks the result of this operation's successful reply.
func (ReconcileParams) DecodeOutput(data []byte) (Unit, error) {
	_, err := decodeOperationResult("reconcile", data, DecodereconcileResult)
	return Unit{}, err
}

// +demi:variant operationResult current_base_version
type currentBaseVersionResult struct {
	Value BaseVersion `json:"value"`
}

// Call identifies the operation and carries its parameters.
func (p CurrentBaseVersionParams) Call() MachineCall { return &CurrentBaseVersion{Params: p} }

// DecodeOutput checks the result of this operation's successful reply.
func (CurrentBaseVersionParams) DecodeOutput(data []byte) (BaseVersion, error) {
	var zero BaseVersion
	value, err := decodeOperationResult("current_base_version", data, DecodecurrentBaseVersionResult)
	if err != nil {
		return zero, err
	}
	return value.Value, nil
}

// +demi:variant operationResult image_state
type imageStateResult struct {
	// +demi:nullable
	Value *MachineImageState `json:"value"`
}

// Call identifies the operation and carries its parameters.
func (p ImageStateParams) Call() MachineCall { return &ImageState{Params: p} }

// DecodeOutput checks the result of this operation's successful reply.
func (ImageStateParams) DecodeOutput(data []byte) (*MachineImageState, error) {
	var zero *MachineImageState
	value, err := decodeOperationResult("image_state", data, DecodeimageStateResult)
	if err != nil {
		return zero, err
	}
	return value.Value, nil
}

// +demi:variant operationResult runtime_state
type runtimeStateResult struct {
	Value RuntimeState `json:"value"`
}

// Call identifies the operation and carries its parameters.
func (p RuntimeStateParams) Call() MachineCall { return &RuntimeStateCall{Params: p} }

// DecodeOutput checks the result of this operation's successful reply.
func (RuntimeStateParams) DecodeOutput(data []byte) (RuntimeState, error) {
	var zero RuntimeState
	value, err := decodeOperationResult("runtime_state", data, DecoderuntimeStateResult)
	if err != nil {
		return zero, err
	}
	return value.Value, nil
}

// +demi:variant operationResult wake
type wakeResult struct {
}

// Call identifies the operation and carries its parameters.
func (p WakeParams) Call() MachineCall { return &Wake{Params: p} }

// DecodeOutput checks the result of this operation's successful reply.
func (WakeParams) DecodeOutput(data []byte) (Unit, error) {
	_, err := decodeOperationResult("wake", data, DecodewakeResult)
	return Unit{}, err
}

// +demi:variant operationResult hibernate
type hibernateResult struct {
}

// Call identifies the operation and carries its parameters.
func (p HibernateParams) Call() MachineCall { return &Hibernate{Params: p} }

// DecodeOutput checks the result of this operation's successful reply.
func (HibernateParams) DecodeOutput(data []byte) (Unit, error) {
	_, err := decodeOperationResult("hibernate", data, DecodehibernateResult)
	return Unit{}, err
}

// +demi:variant operationResult checkpoint
type checkpointResult struct {
}

// Call identifies the operation and carries its parameters.
func (p CheckpointParams) Call() MachineCall { return &Checkpoint{Params: p} }

// DecodeOutput checks the result of this operation's successful reply.
func (CheckpointParams) DecodeOutput(data []byte) (Unit, error) {
	_, err := decodeOperationResult("checkpoint", data, DecodecheckpointResult)
	return Unit{}, err
}

// +demi:variant operationResult grow_volume
type growVolumeResult struct {
}

// Call identifies the operation and carries its parameters.
func (p GrowVolumeParams) Call() MachineCall { return &GrowVolume{Params: p} }

// DecodeOutput checks the result of this operation's successful reply.
func (GrowVolumeParams) DecodeOutput(data []byte) (Unit, error) {
	_, err := decodeOperationResult("grow_volume", data, DecodegrowVolumeResult)
	return Unit{}, err
}

// +demi:variant operationResult reset
type resetResult struct {
}

// Call identifies the operation and carries its parameters.
func (p ResetParams) Call() MachineCall { return &Reset{Params: p} }

// DecodeOutput checks the result of this operation's successful reply.
func (ResetParams) DecodeOutput(data []byte) (Unit, error) {
	_, err := decodeOperationResult("reset", data, DecoderesetResult)
	return Unit{}, err
}
