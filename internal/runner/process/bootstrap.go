package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

// RunChildBootstrap handles the private child-launch mode before the runner
// starts services, changes limits, or dispatches command aliases. On Unix it
// applies child attributes in a separate copy of the runner and replaces that
// process with the requested executable. This avoids changing the parent
// process's umask or limits and needs no cgo or unsafe fork callback.
//
// Every runner entry point that uses Wrap must call this first. A true result
// means the invocation was a bootstrap and the caller must exit, reporting err.
// Ordinary invocations (and Windows) return false. Successful Unix execution
// replaces the process and does not return.
func RunChildBootstrap() (handled bool, err error) { panic("not written: r-process") }
