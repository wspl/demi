package database

// Written describes what a plugin value write did.
//
//sumtype:decl
type Written interface{ written() }

// WrittenRevision holds the new revision, or the removed revision for a delete.
type WrittenRevision struct{ Revision uint64 }

func (*WrittenRevision) written() {}

// WrittenConflict means another write came first.
type WrittenConflict struct{}

func (*WrittenConflict) written() {}

// WrittenRefused means a touched blob is being deleted and nothing was written.
type WrittenRefused struct{ Err error }

func (*WrittenRefused) written() {}
