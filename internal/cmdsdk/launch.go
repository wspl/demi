package cmdsdk

import "fmt"

// CommandService is the sole argument used to launch native command services.
const CommandService = "--command-service"

// Launch is the validated native service launch mode.
type Launch struct{}

// ParseLaunch accepts only --command-service, without help or version flags.
func ParseLaunch(args []string) (Launch, error) {
	if len(args) != 1 || args[0] != CommandService {
		return Launch{}, fmt.Errorf("usage: %s", CommandService)
	}
	return Launch{}, nil
}
