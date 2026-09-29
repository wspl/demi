package backend

import "github.com/wspl/demi/go/core"

// Services is what the users share, safe from any goroutine: the program
// (package edge's start) builds it once and hands it to the shards
// (g7-backend.md § Services). Storage, usage, auth, the vault, the model
// access, the managed Clouds and the synchronization registry join it with
// their parts.
type Services struct {
	// Clock is the wall clock the backend reads times from.
	Clock core.Clock
}
