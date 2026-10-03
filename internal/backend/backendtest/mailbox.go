package backendtest

//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/backend/accounts"
)

// Mailbox captures verification mail, or refuses it while failing is set.
// Its zero value is ready; all methods support concurrent use.
type Mailbox struct{}

// SetFailing makes subsequent deliveries fail with the fixture transport error.
func (m *Mailbox) SetFailing(failing bool) { panic("not written: b-backend") }

// SendVerification captures mail or returns the scripted transport failure.
func (m *Mailbox) SendVerification(ctx context.Context, mail accounts.VerificationMail) error {
	panic("not written: b-backend")
}

// Sent returns a snapshot of delivered verification messages, in send order.
func (m *Mailbox) Sent() []accounts.VerificationMail { panic("not written: b-backend") }

var _ accounts.AccountMail = (*Mailbox)(nil)
