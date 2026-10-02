package database

import (
	"github.com/wspl/demi/internal/webapi"
)

// ChallengeOutcome describes how a confirmation ended.
//
//sumtype:decl
type ChallengeOutcome interface{ challengeOutcome() }

// ChallengeChanged means the account has the new address and the challenge is consumed.
type ChallengeChanged struct{ User webapi.UserDTO }

func (*ChallengeChanged) challengeOutcome() {}

// ChallengeInvalidCode means the code is wrong, expired, used up, or no longer holds.
type ChallengeInvalidCode struct{}

func (*ChallengeInvalidCode) challengeOutcome() {}

// ChallengeEmailTaken means another account took the address after issue.
type ChallengeEmailTaken struct{}

func (*ChallengeEmailTaken) challengeOutcome() {}
