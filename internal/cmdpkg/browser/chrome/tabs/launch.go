package tabs

import (
	"bufio"
	"context"
	"embed"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserproto"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/contract"
)

//go:embed extension/background.js extension/manifest.json extension/offscreen.html extension/offscreen.js
var captureExtension embed.FS

const captureExtensionID = "ekadkclcinpnbbdeloemlmaimcklplko"

// desktopUserAgent keeps Chrome's platform and reduced major version without headless markers.
func desktopUserAgent(version string) string {
	platform := "X11; Linux x86_64"
	switch runtime.GOOS {
	case "darwin":
		platform = "Macintosh; Intel Mac OS X 10_15_7"
	case "windows":
		platform = "Windows NT 10.0; Win64; x64"
	}
	major, _, _ := strings.Cut(version, ".")
	return fmt.Sprintf(
		"Mozilla/5.0 (%s) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/%s.0.0.0 Safari/537.36",
		platform,
		major,
	)
}

// configureLaunch writes the extension and locale before Chrome can create a renderer.
func configureLaunch(options LaunchOptions, profile, capture string) (*exec.Cmd, error) {
	extension := filepath.Join(profile, "demi-capture")
	if err := os.Mkdir(extension, 0o700); err != nil {
		return nil, err
	}
	for _, name := range []string{"manifest.json", "background.js", "offscreen.html", "offscreen.js"} {
		data, err := captureExtension.ReadFile("extension/" + name)
		if err != nil {
			return nil, err
		}
		if err := os.WriteFile(filepath.Join(extension, name), data, 0o600); err != nil {
			return nil, err
		}
	}
	socket, err := contract.EncodeJSON(capture)
	if err != nil {
		return nil, err
	}
	codec, err := contract.EncodeJSON(browserproto.VideoCodec)
	if err != nil {
		return nil, err
	}
	if err := os.WriteFile(
		filepath.Join(extension, "config.js"),
		fmt.Appendf(nil, "export const socket = %s;\nexport const codec = %s;\n", socket, codec),
		0o600,
	); err != nil {
		return nil, err
	}
	preferences := filepath.Join(profile, "Default")
	if err := os.Mkdir(preferences, 0o700); err != nil {
		return nil, err
	}
	// This Chrome preference is private launch configuration, not a Demi contract.
	configuration := struct {
		Intl struct {
			Languages string `json:"accept_languages"`
		} `json:"intl"`
	}{}
	configuration.Intl.Languages = strings.Join(localeLanguages(options.Locale), ",")
	encoded, err := contract.EncodeJSON(configuration)
	if err != nil {
		return nil, err
	}
	if err := os.WriteFile(filepath.Join(preferences, "Preferences"), encoded, 0o600); err != nil {
		return nil, err
	}
	version, err := PinnedVersion()
	if err != nil {
		return nil, err
	}
	args := chromeArguments(version)
	args = append(args, "--user-data-dir="+profile, "--remote-debugging-port=0", "--load-extension="+extension)
	if runtime.GOOS == "darwin" && len(options.Locale.Languages) > 0 {
		args = append(args, "-AppleLanguages", "("+string(options.Locale.Languages[0])+")")
	}
	command := exec.Command(options.Executable, args...)
	command.Env = chromeEnvironment(options.Locale)
	return command, nil
}

// chromeArguments is the pinned native launch policy, excluding per-environment paths.
func chromeArguments(version string) []string {
	args := []string{
		"--disable-background-networking",
		"--disable-background-timer-throttling",
		"--disable-backgrounding-occluded-windows",
		"--disable-breakpad",
		"--disable-client-side-phishing-detection",
		"--disable-component-extensions-with-background-pages",
		"--disable-default-apps",
		"--disable-dev-shm-usage",
		"--disable-hang-monitor",
		"--disable-ipc-flooding-protection",
		"--disable-popup-blocking",
		"--disable-prompt-on-repost",
		"--disable-renderer-backgrounding",
		"--disable-sync",
		"--metrics-recording-only",
		"--no-first-run",
		"--use-mock-keychain",
		"--no-startup-window",
		"--headless",
		"--mute-audio",
		"--enable-features=NetworkService,NetworkServiceInProcess",
		"--disable-features=ReduceAcceptLanguage,TranslateUI,InitialWebUI," +
			"WebUIToolbarProcessOverheadExperiment,PreloadTopChromeWebUI,WebUIOmniboxPopup," +
			"WebUIOmniboxAimPopup,ExtensionDisableUnsupportedDeveloper",
		"--force-color-profile=srgb",
		"--password-store=basic",
		"--enable-blink-features=IdleDetection",
		"--disable-blink-features=AutomationControlled",
		"--user-agent=" + desktopUserAgent(version),
		"--allowlisted-extension-id=" + captureExtensionID,
	}
	if runtime.GOOS == "linux" {
		args = append(
			args,
			"--blink-settings=primaryPointerType=4,availablePointerTypes=4,primaryHoverType=2,"+
				"availableHoverTypes=2",
		)
	}
	return args
}

// chromeEnvironment overrides only variables Chrome needs for the user's locale.
func chromeEnvironment(locale cmdproto.CommandLocale) []string {
	overrides := map[string]string{"TZ": locale.TimeZone}
	if runtime.GOOS == "linux" && len(locale.Languages) > 0 {
		system := strings.ReplaceAll(string(locale.Languages[0]), "-", "_") + ".UTF-8"
		overrides["LANG"] = system
		overrides["LC_ALL"] = system
		overrides["LANGUAGE"] = strings.Join(localeLanguages(locale), ":")
	}
	env := os.Environ()
	for key, value := range overrides {
		prefix := key + "="
		kept := env[:0]
		for _, entry := range env {
			if !strings.HasPrefix(entry, prefix) {
				kept = append(kept, entry)
			}
		}
		env = append(kept, prefix+value)
	}
	return env
}

// startChrome retains and drains stderr while waiting for Chrome's CDP endpoint.
func startChrome(
	ctx context.Context,
	command *exec.Cmd,
	directories *environmentDirectories,
) (*chromeProcess, string, func(), error) {
	reader, writer, err := os.Pipe()
	if err != nil {
		return nil, "", nil, err
	}
	command.Stderr = writer
	endpoint := make(chan string, 1)
	refused := make(chan struct{})
	drained := make(chan struct{})
	go func() {
		scanChromeLogs(reader, endpoint, refused, drained)
	}()
	stopLogs := func() {
		_ = reader.Close()
		<-drained
	} // Closing the owned pipe wakes its scanner.
	process := &chromeProcess{runtime: directories.runtime, installations: []string{installationOf(command.Path)}}
	err = process.start(command, directories.profile)
	closeErr := writer.Close()
	if err != nil {
		stopLogs()
		return nil, "", nil, errors.Join(err, closeErr)
	}
	bounded, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	select {
	case address := <-endpoint:
		return process, address, stopLogs, nil
	case <-refused:
		err = &cdp.BrowserError{Kind: cdp.KindRoot}
	case <-process.done:
		// Exit may become ready before the stderr reader runs. Preserve the
		// refusal already in the pipe, using the existing startup deadline.
		select {
		case <-refused:
			return process, "", stopLogs, &cdp.BrowserError{Kind: cdp.KindRoot}
		case <-drained:
		case <-bounded.Done():
			return process, "", stopLogs, bounded.Err()
		}
		select {
		case <-refused:
			err = &cdp.BrowserError{Kind: cdp.KindRoot}
		default:
			err = &cdp.BrowserError{
				Kind:    cdp.KindUnavailable,
				Message: "Chrome exited before its debugging endpoint was ready",
				Cause:   process.waitErr,
			}
		}
	case <-bounded.Done():
		err = bounded.Err()
	}
	return process, "", stopLogs, err
}

// localeLanguages converts the checked language tags for Chrome's native settings.
func localeLanguages(locale cmdproto.CommandLocale) []string {
	languages := make([]string, len(locale.Languages))
	for index, language := range locale.Languages {
		languages[index] = string(language)
	}
	return languages
}

func scanChromeLogs(reader *os.File, endpoint chan<- string, refused chan struct{}, drained chan struct{}) {
	defer close(drained)
	scan := bufio.NewReader(reader)
	for {
		line, readErr := scan.ReadString('\n')
		if strings.Contains(line, "Running as root without --no-sandbox is not supported") {
			select {
			case <-refused:
			default:
				close(refused)
			}
		}
		line = strings.TrimSpace(line)
		if address, ok := strings.CutPrefix(line, "DevTools listening on "); ok {
			select {
			case endpoint <- address:
			default:
			}
		}
		if readErr != nil {
			return
		}
	}
}
