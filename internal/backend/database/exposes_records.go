package database

import (
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// ExposeRecord is an `exposes` row.
type ExposeRecord struct {
	ID        webapiproto.ExposeID
	User      webapiproto.UserID
	Device    webapiproto.DeviceID
	Address   webapiproto.ExposeAddress
	CreatedAt types.Timestamp
	ExpiresAt types.Timestamp
}

// UserExposes is a user's exposes as a listing leaves them: the live ones, soonest expiry
// first, and the ids of the expired ones, which it deleted.
type UserExposes struct {
	Live    []ExposeRecord
	Expired []webapiproto.ExposeID
}
