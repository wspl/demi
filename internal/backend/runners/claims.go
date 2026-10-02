package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/runnerwire"
	"github.com/wspl/demi/internal/webapi"
)

// PendingClaims holds runners waiting to be paired and each user's recent claims.
// Construct it with NewPendingClaims. Its methods are safe for concurrent use.
// Close releases every pending wait at backend shutdown.
type PendingClaims struct{}

// NewPendingClaims sets the number of attempts each user may make in one minute.
func NewPendingClaims(attemptsPerMinute int) *PendingClaims { panic("not written: b-runners") }

// Attempt counts a claim attempt unless the user has exhausted the minute's
// allowance; then it returns false and counts nothing.
func (p *PendingClaims) Attempt(user webapi.UserID) bool { panic("not written: b-runners") }

// Take removes the runner under code so only this claim holds it, or returns nil.
// The caller must defer the returned runner's Release, including after Grant.
func (p *PendingClaims) Take(code ClaimCode) *PendingRunner { panic("not written: b-runners") }

// Register puts runner up for claiming under code; nil means shutdown has begun.
// The caller defers the wait's Release and Withdraw(code) when its socket leaves.
func (p *PendingClaims) Register(code ClaimCode, runner runnerwire.RunnerInfo) *ClaimWait {
	panic("not written: b-runners")
}

// Withdraw takes a code back when it expires or its runner goes away.
func (p *PendingClaims) Withdraw(code ClaimCode) { panic("not written: b-runners") }

// Close lets every waiting runner go and accepts no further registrations.
func (p *PendingClaims) Close() { panic("not written: b-runners") }

// ClaimWait owns the waiting runner's end of a claim. Do not copy it.
type ClaimWait struct{}

// Wait receives the grant, or nil if the claim was withdrawn or abandoned.
// Context cancellation returns an error. A received grant must be released.
func (w *ClaimWait) Wait(ctx context.Context) (*ClaimGrant, error) { panic("not written: b-runners") }

// Release idempotently abandons the runner's wait and any undelivered grant.
func (w *ClaimWait) Release() { panic("not written: b-runners") }

// PendingRunner is a waiting runner taken by one claim. Do not copy it.
type PendingRunner struct {
	// Runner is the information reported by the waiting runner.
	Runner runnerwire.RunnerInfo
}

// Grant hands the runner its device and token without waiting for socket binding.
// The caller owns the returned answer and must defer its Release.
func (p *PendingRunner) Grant(device database.DeviceRecord, token runnerwire.DeviceToken) *ClaimAnswer {
	panic("not written: b-runners")
}

// Release idempotently abandons an ungranted claim; it does nothing after Grant.
func (p *PendingRunner) Release() { panic("not written: b-runners") }

// ClaimGrant hands a runner its device and private token, and owns the reply
// to the claimant. The runner defers Release and calls Bound after socket binding.
type ClaimGrant struct {
	// Device is the device created by the claim.
	Device database.DeviceRecord
	// Token is the credential only the runner receives.
	Token runnerwire.DeviceToken
}

// Bound completes the claimant's answer with the bound device, without waiting.
func (g *ClaimGrant) Bound(device webapi.DeviceDTO) { panic("not written: b-runners") }

// Release idempotently abandons an unanswered grant, waking its claimant.
func (g *ClaimGrant) Release() { panic("not written: b-runners") }

// ClaimAnswer owns the claimant's wait for the runner to bind its socket.
type ClaimAnswer struct{}

// Wait returns the bound device, or nil when the runner left before binding.
// Context cancellation returns an error.
func (a *ClaimAnswer) Wait(ctx context.Context) (*webapi.DeviceDTO, error) {
	panic("not written: b-runners")
}

// Release idempotently abandons the claimant's answer without blocking the runner.
func (a *ClaimAnswer) Release() { panic("not written: b-runners") }
