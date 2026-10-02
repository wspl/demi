// Copyright (c) 2026, the Demi contributors
// See LICENSE for licensing information

package interp

import "sync"

// taskGroup owns an interpreter scope's tasks and registers them with all
// ancestor scopes, so joining the job includes work started by nested shells.
// A task remains registered until its function and resource cleanup return.
type taskGroup struct {
	parent *taskGroup
	wg     sync.WaitGroup
}

func (g *taskGroup) Go(f func()) {
	for scope := g; scope != nil; scope = scope.parent {
		scope.wg.Add(1)
	}
	go func() {
		defer func() {
			for scope := g; scope != nil; scope = scope.parent {
				scope.wg.Done()
			}
		}()
		f()
	}()
}

func (g *taskGroup) Wait() { g.wg.Wait() }
