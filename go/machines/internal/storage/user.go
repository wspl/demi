package storage

// UserID is the UID and GID of the sandbox's user, demi. It has no build tag:
// the sandbox package writes it into files and the configuration on every
// platform.
const UserID = 1000
