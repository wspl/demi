package database

import (
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// DeviceRecord is a `devices` row.
type DeviceRecord struct {
	ID         webapiproto.DeviceID
	User       webapiproto.UserID
	Kind       webapiproto.DeviceKind
	Name       string
	Platform   runnerproto.RunnerPlatform
	ClaimedAt  types.Timestamp
	LastSeenAt *types.Timestamp
}
