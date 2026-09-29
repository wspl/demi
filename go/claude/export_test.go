package claude

// NewLoopbackInstaller returns an installer whose releases may also name plain
// HTTP on 127.0.0.1, for a test's fixture server.
func NewLoopbackInstaller(roots Roots) *Installer {
	installer := NewInstaller(roots)
	installer.allowLoopbackHTTP = true
	return installer
}
