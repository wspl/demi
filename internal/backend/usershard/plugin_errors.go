package usershard

import (
	"errors"
	"fmt"
	"math"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
)

func exposeFailure(err error) error {
	if err == nil {
		return nil
	}
	var offline *expose.DeviceOfflineError
	var missing *expose.NotFoundError
	var reason plugin.ExposeRefusal
	switch {
	case errors.Is(err, expose.ErrUnavailable):
		reason = plugin.ExposeRefusalUnavailable
	case errors.Is(err, expose.ErrDeviceNotFound):
		reason = plugin.ExposeRefusalDeviceNotFound
	case errors.As(err, &offline):
		reason = plugin.ExposeRefusalDeviceOffline
	case errors.As(err, &missing):
		reason = plugin.ExposeRefusalNotFound
	default:
		return err
	}
	return &plugin.PortRefusalExpose{Reason: reason, Message: err.Error()}
}

func accessFailure(err error) error {
	var access *hostaccess.Error
	if errors.As(err, &access) {
		code, status := access.Code()
		return &plugin.PortRefusalHost{
			Code:    code,
			Status:  uint16(status),
			Message: access.Error(),
		}
	}
	return err
}

func callFailure(err error) error {
	var call *remotehost.ServiceCallError
	if errors.As(err, &call) {
		if call.Kind == remotehost.ServiceExited {
			return &plugin.PortRefusalOperation{Stderr: call.Stderr}
		}
		var remote *host.Error
		if errors.As(call, &remote) {
			return accessFailure(&hostaccess.Error{Kind: hostaccess.AccessHost, Cause: remote})
		}
	}
	return accessFailure(err)
}

// exposeLifetime converts a plugin's expose lifetime without wrapping nanoseconds.
func exposeLifetime(seconds uint64) (time.Duration, error) {
	const maximum = uint64(math.MaxInt64 / int64(time.Second))
	if seconds > maximum {
		err := &database.Error{
			Kind: database.TimeRange,
			Err:  fmt.Errorf("expose lifetime exceeds %d seconds", maximum),
		}
		return 0, &host.PortError{Kind: host.PortFailed, Message: err.Error(), Err: err}
	}
	return time.Duration(seconds) * time.Second, nil
}
