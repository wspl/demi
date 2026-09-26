package storage

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

// TargetKind names where a conversation runs.
type TargetKind string

// The target kinds.
const (
	TargetCloud     TargetKind = "cloud"
	TargetDevice    TargetKind = "device"
	TargetWorkspace TargetKind = "workspace"
)

// ConversationTarget is a conversation's persisted target selection
// (conversations.target_json). Which fields a target has depends on its kind:
//
//	cloud      Path, optional, absolute
//	device     DeviceID and Path, both required
//	workspace  WorkspaceID, required
//
// The field order is the stored key order; a target switch compares stored
// text, so the encoding must stay byte-identical to the TypeScript backend's.
type ConversationTarget struct {
	Kind        TargetKind `json:"kind"`
	DeviceID    string     `json:"deviceId,omitempty"`
	WorkspaceID string     `json:"workspaceId,omitempty"`
	Path        string     `json:"path,omitempty"`
}

// Validate reports whether the target has exactly the fields its kind allows.
func (t ConversationTarget) Validate() error {
	switch t.Kind {
	case TargetCloud:
		if t.DeviceID != "" || t.WorkspaceID != "" {
			return errors.New("a cloud target names no device or workspace")
		}
		if t.Path != "" && !strings.HasPrefix(t.Path, "/") {
			return fmt.Errorf("cloud target path %q is not absolute", t.Path)
		}
	case TargetDevice:
		if t.DeviceID == "" || t.Path == "" {
			return errors.New("a device target needs a device id and a path")
		}
		if t.WorkspaceID != "" {
			return errors.New("a device target names no workspace")
		}
	case TargetWorkspace:
		if t.WorkspaceID == "" {
			return errors.New("a workspace target needs a workspace id")
		}
		if t.DeviceID != "" || t.Path != "" {
			return errors.New("a workspace target names no device or path")
		}
	default:
		return fmt.Errorf("unknown target kind %q", t.Kind)
	}
	return nil
}

// parseConversationTarget reads a stored target: unknown keys and fields the
// kind does not allow are errors.
func parseConversationTarget(text string) (ConversationTarget, error) {
	var target ConversationTarget
	decoder := json.NewDecoder(strings.NewReader(text))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&target); err != nil {
		return ConversationTarget{}, fmt.Errorf("conversation target: %w", err)
	}
	if err := target.Validate(); err != nil {
		return ConversationTarget{}, fmt.Errorf("conversation target: %w", err)
	}
	return target, nil
}

// ExecutionTarget is where a conversation actually ran on one side of a
// target switch: the resolved device and directory. A cloud target's device
// is null until the user has a Cloud device.
type ExecutionTarget struct {
	Kind        TargetKind `json:"kind"`
	WorkspaceID string     `json:"workspaceId,omitempty"`
	DeviceID    *string    `json:"deviceId"`
	Path        string     `json:"path"`
}

// executionTargetFields are the stored keys; unknown keys are dropped, as the
// schema the TypeScript backend reads them with does.
type executionTargetFields struct {
	Kind        *TargetKind     `json:"kind"`
	WorkspaceID *string         `json:"workspaceId"`
	DeviceID    json.RawMessage `json:"deviceId"`
	Path        *string         `json:"path"`
}

// UnmarshalJSON reads an execution target, requiring the fields of its kind.
func (t *ExecutionTarget) UnmarshalJSON(data []byte) error {
	var fields executionTargetFields
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	if fields.Kind == nil || fields.Path == nil || fields.DeviceID == nil {
		return errors.New("execution target needs kind, deviceId and path")
	}
	var deviceID *string
	if err := json.Unmarshal(fields.DeviceID, &deviceID); err != nil {
		return fmt.Errorf("execution target deviceId: %w", err)
	}
	parsed := ExecutionTarget{Kind: *fields.Kind, DeviceID: deviceID, Path: *fields.Path}
	switch parsed.Kind {
	case TargetCloud:
	case TargetDevice:
		if deviceID == nil {
			return errors.New("a device execution target needs a device id")
		}
	case TargetWorkspace:
		if deviceID == nil || fields.WorkspaceID == nil {
			return errors.New("a workspace execution target needs a workspace and a device id")
		}
		parsed.WorkspaceID = *fields.WorkspaceID
	default:
		return fmt.Errorf("unknown execution target kind %q", parsed.Kind)
	}
	*t = parsed
	return nil
}

// TargetSwitch is the latest explicit target switch, kept for every node's
// execution context.
type TargetSwitch struct {
	From ExecutionTarget `json:"from"`
	To   ExecutionTarget `json:"to"`
}

// jsonText encodes v as JavaScript's JSON.stringify does for the values
// storage writes: no HTML escaping, no trailing newline. Encoding/json still
// escapes U+2028 and U+2029, which JSON.stringify leaves as they are.
func jsonText(v any) (string, error) {
	var buffer bytes.Buffer
	encoder := json.NewEncoder(&buffer)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(v); err != nil {
		return "", err
	}
	return strings.TrimSuffix(buffer.String(), "\n"), nil
}
