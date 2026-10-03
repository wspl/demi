package edge

import "github.com/wspl/demi/internal/backend/usershard"

// AppState is what the routes reach: the services every request may use, the
// users' shards, and how the backend is reached from outside. All three handles
// are required and remain valid until the edge closes.
type AppState struct {
	// Services are shared by every request and shard.
	Services *usershard.Services
	// Shards routes requests to the owning user's runtime.
	Shards *usershard.Shards
	// Site describes the public origin and installation downloads.
	Site *Site
}
