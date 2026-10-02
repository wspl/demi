package database

import (
	"github.com/wspl/demi/internal/webapi"
)

// ExecutionDeviceID returns the target's device, nil before a Cloud's first use.
func ExecutionDeviceID(target ExecutionTarget) *webapi.DeviceID {
	switch target := target.(type) {
	case *ExecutionCloud:
		return target.DeviceID
	case *ExecutionDevice:
		return &target.DeviceID
	case *ExecutionWorkspace:
		return &target.DeviceID
	}
	return nil
}

// ExecutionPath returns the directory work starts in on the target.
func ExecutionPath(target ExecutionTarget) string {
	switch target := target.(type) {
	case *ExecutionCloud:
		return target.Path
	case *ExecutionDevice:
		return target.Path
	case *ExecutionWorkspace:
		return target.Path
	}
	return ""
}
