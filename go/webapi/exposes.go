package webapi

import (
	"github.com/wspl/demi/go/core"
)

// An expose as every surface shows it: its public `url` and when it ends.
//
//demi:wire open
type ExposeDTO struct {
	ID ExposeID `json:"id" check:"func=Validate"`
	// Its number among the user's exposes, which the commands take and
	// print (`expose.md` § The expose record).
	Number    uint64         `json:"number" check:"range=1..9007199254740991"`
	DeviceID  DeviceID       `json:"deviceId" check:"func=Validate"`
	Address   ExposeAddress  `json:"address" check:"func=Validate"`
	URL       string         `json:"url"`
	CreatedAt core.Timestamp `json:"createdAt" check:"func=core.Validate"`
	ExpiresAt core.Timestamp `json:"expiresAt" check:"func=core.Validate"`
}

// `GET /exposes`: the caller's live exposes, soonest expiry first.
//
//demi:wire open
type Exposes struct {
	Exposes []ExposeDTO `json:"exposes"`
}

// `{ expose }`: the answer of a creation and a renewal.
//
//demi:wire open
type ExposeAnswer struct {
	Expose ExposeDTO `json:"expose"`
}

// `POST /exposes`: a service on one of the caller's connected devices.
//
//demi:wire
type CreateExpose struct {
	DeviceID DeviceID `json:"deviceId" check:"func=Validate"`
	// An `ExposeAddress` is valid once it is decoded.
	Address ExposeAddress `json:"address" check:"func=Validate"`
}
