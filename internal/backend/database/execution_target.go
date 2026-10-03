package database

import (
	"github.com/wspl/demi/internal/webapi"
)

// ExecutionDeviceID returns the target's device; false before a Cloud's first use.
func ExecutionDeviceID(target ExecutionTarget) (webapi.DeviceID, bool) {
	switch target := target.(type) {
	case *ExecutionCloud:
		if target.DeviceID == nil {
			return "", false
		}
		return *target.DeviceID, true
	case *ExecutionDevice:
		return target.DeviceID, true
	case *ExecutionWorkspace:
		return target.DeviceID, true
	}
	return "", false
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
