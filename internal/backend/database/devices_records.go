package database

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// DeviceRecord is a `devices` row.
type DeviceRecord struct {
	ID         webapi.DeviceID
	User       webapi.UserID
	Kind       webapi.DeviceKind
	Name       string
	Platform   runnerwire.RunnerPlatform
	ClaimedAt  core.Timestamp
	LastSeenAt *core.Timestamp
}
