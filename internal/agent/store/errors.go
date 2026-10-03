package store

import "errors"

// ErrInvalidated means a history rewrite or dispose made the save stale.
var ErrInvalidated = errors.New("the command storage handle is no longer current")

// ErrCorrupt means stored data is not a valid record or checkpoint.
var ErrCorrupt = errors.New("the stored agent tree is corrupt")
