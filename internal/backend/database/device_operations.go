package database

import (
	"github.com/wspl/demi/internal/webapi"
)

// DeviceOperation is an unfinished reset with the device it belongs to.
type DeviceOperation struct {
	Device    webapi.DeviceID
	Operation ManagedOperation
}
