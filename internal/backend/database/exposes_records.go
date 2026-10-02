package database

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// ExposeRecord is an `exposes` row.
type ExposeRecord struct {
	ID        webapi.ExposeID
	User      webapi.UserID
	Device    webapi.DeviceID
	Address   webapi.ExposeAddress
	CreatedAt core.Timestamp
	ExpiresAt core.Timestamp
}

// UserExposes is a user's exposes as a listing leaves them: the live ones, soonest expiry
// first, and the ids of the expired ones, which it deleted.
type UserExposes struct {
	Live    []ExposeRecord
	Expired []webapi.ExposeID
}
