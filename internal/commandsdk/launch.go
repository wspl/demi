package commandsdk

import "fmt"

// CommandService is the sole argument used to launch native command services.
const CommandService = "--command-service"

// CheckLaunch accepts only --command-service, without help or version flags.
func CheckLaunch(args []string) error {
	if len(args) != 1 || args[0] != CommandService {
		return fmt.Errorf("usage: %s", CommandService)
	}
	return nil
}
