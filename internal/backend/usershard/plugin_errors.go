package usershard

import (
	"errors"
	"fmt"

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
		return &plugin.PortRefusalHost{Code: code, Status: uint16(status), Message: access.Error()}
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

// exposeLifetimeRangeError reports the duration representation gap at the expose API.
type exposeLifetimeRangeError struct{ Seconds uint64 }

func (e *exposeLifetimeRangeError) Error() string {
	return fmt.Sprintf("expose lifetime %d seconds exceeds the Go expose API duration range", e.Seconds)
}
