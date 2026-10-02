package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"github.com/wspl/demi/internal/webapi"
)

// ExecutionDeviceID returns the target's device, nil before a Cloud's first use.
func ExecutionDeviceID(target ExecutionTarget) *webapi.DeviceID { panic("not written: b-database") }

// ExecutionPath returns the directory work starts in on the target.
func ExecutionPath(target ExecutionTarget) string { panic("not written: b-database") }
