package artifact

// syncDirectory does nothing: Windows cannot open a directory as a file, so
// there a directory's entries reach the disk when the system writes them.
func syncDirectory(string) error { return nil }
