package grokbuild

import (
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

// The Grok Build family's secret document: the OAuth tokens and their
// expiry, the issuer and client that issued them, the team or organization
// the tokens act for, and the user they belong to. Demi defines it; it is
// not the Grok CLI's `auth.json`.
// +demi:root
type secret struct {
	AccessToken  provider.Secret  `json:"accessToken"`
	RefreshToken *provider.Secret `json:"refreshToken,omitempty"`
	ExpiresAt    *types.Timestamp `json:"expiresAt,omitempty"`
	// Such as `https://auth.x.ai`; refreshes go to its `/oauth2/token`.
	Issuer    issuer     `json:"issuer"`
	ClientID  string     `json:"clientId"`
	Principal *principal `json:"principal,omitempty"`
	UserID    *string    `json:"userId,omitempty"`
	Email     *string    `json:"email,omitempty"`
}

// Whom the tokens act for, such as a team: its type and id.
// +demi:root
type principal struct {
	Kind string `json:"kind"`
	ID   string `json:"id"`
}

// The issuer of a sign-in: an `http` or `https` URL, written as its text.
// +demi:format http-url
// +demi:id
type issuer string
