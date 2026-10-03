package database

import "errors"

// ErrRevisionConflict means another write changed the plugin value first.
var ErrRevisionConflict = errors.New("the plugin value is not at the given revision")
