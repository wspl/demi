package runnerstest

import (
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/webapiproto"
)

// LastSeen builds last-seen persistence for a user's test connections. Its private
// marks registry starts no workers and holds no registrations. The connection's
// Serve owner joins the persistence work before the control service closes.
func LastSeen(control *database.ControlService, user webapiproto.UserID) *runners.LastSeen {
	return runners.NewLastSeen(control, (&pagesync.Registry{}).Of(user))
}
