package database

import (
	"github.com/wspl/demi/internal/webapiproto"
)

// DeviceOperation is an unfinished reset with the device it belongs to.
type DeviceOperation struct {
	Device    webapiproto.DeviceID
	Operation ManagedOperation
}
