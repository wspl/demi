package main

import "os/exec"

// Cross builds are run on the developer's Mac. On Windows CommandContext owns
// the direct Go child; native process-group shutdown is used by Unix builders.
func ownBuildProcess(_ *exec.Cmd) {}
