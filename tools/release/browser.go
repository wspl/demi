package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/contract"
)

type chromePlatform struct{ Name, Target, Executable string }

var chromePlatforms = []chromePlatform{
	{
		"mac-arm64",
		"aarch64-apple-darwin",
		"chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
	},
	{
		"mac-x64",
		"x86_64-apple-darwin",
		"chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
	},
	{"linux-arm64", "aarch64-unknown-linux-musl", "chrome-linux-arm64/chrome"},
	{"linux64", "x86_64-unknown-linux-musl", "chrome-linux64/chrome"},
	{"win64", "x86_64-pc-windows-msvc", "chrome-win64/chrome.exe"},
}

func (a *application) browser(ctx context.Context, version string) error {
	client := artifacts.NewClient()
	defer client.Close()
	data, err := a.prepareBrowser(
		ctx,
		client,
		"https://googlechromelabs.github.io/chrome-for-testing",
		"https://storage.googleapis.com/",
		version,
	)
	if err != nil {
		return err
	}
	destination := filepath.Join(a.Root, "internal/cmdpkg/browser/browserop/chrome.json")
	if err := artifacts.PublishBytes(
		ctx,
		destination,
		data,
		artifacts.Publication{Mode: artifacts.Replace, Durable: true},
	); err != nil {
		return err
	}
	_, err = fmt.Fprintf(a.Out, "Chrome for Testing %s: %s\n", version, destination)
	return err
}

func (a *application) prepareBrowser(
	ctx context.Context,
	client *artifacts.Client,
	metadata, official, version string,
) (data []byte, err error) {
	release := browserop.BrowserRelease{Version: version, Platforms: []browserop.ReleasePlatform{}}
	if err := release.Validate(); err != nil {
		return nil, err
	}
	var body bytes.Buffer
	if _, err := artifacts.DownloadMeasured(ctx, client, metadata+"/"+version+".json", 1024*1024, &body); err != nil {
		return nil, err
	}
	entries, err := chromeDownloads(body.Bytes(), version)
	if err != nil {
		return nil, err
	}
	dir, err := os.MkdirTemp("", "demi-chrome-")
	if err != nil {
		return nil, err
	}
	defer func() { err = errors.Join(err, os.RemoveAll(dir)) }()
	for _, platform := range chromePlatforms {
		location := ""
		found := false
		for _, entry := range entries {
			if *entry.Platform == platform.Name {
				location = *entry.URL
				found = true
				break
			}
		}
		if !found {
			//nolint:staticcheck // ST1005: product text, shown to the user as written.
			return nil, fmt.Errorf("Chrome for Testing %s has no %s archive", version, platform.Name)
		}
		if !strings.HasPrefix(location, official) {
			return nil, fmt.Errorf("the %s archive is not an official download: %s", platform.Name, location)
		}
		entry, digest, err := measureChromeArchive(ctx, client, dir, location, platform)
		if err != nil {
			return nil, err
		}
		release.Platforms = append(release.Platforms, entry)
		if _, err := fmt.Fprintf(
			a.Out,
			"Chrome for Testing %s: %s archive, %d bytes\n",
			version,
			platform.Name,
			digest.Size,
		); err != nil {
			return nil, err
		}
	}
	if err := release.Validate(); err != nil {
		return nil, err
	}
	return record(release)
}

type chromeDownload struct {
	Platform *string `json:"platform"`
	URL      *string `json:"url"`
}

func chromeDownloads(data []byte, version string) ([]chromeDownload, error) {
	// Vendor metadata is decoded in two stages so absent or null required fields
	// cannot turn into valid Go zero values. Unread vendor fields remain opaque.
	var envelope struct {
		Version   *string         `json:"version"`
		Downloads json.RawMessage `json:"downloads"`
	}
	if err := contract.CheckJSON(data); err != nil {
		return nil, err
	}
	if err := json.Unmarshal(data, &envelope); err != nil {
		return nil, err
	}
	if envelope.Version == nil {
		return nil, fmt.Errorf("the metadata of Chrome for Testing %s is invalid: missing version", version)
	}
	if *envelope.Version != version {
		return nil, fmt.Errorf(
			"the metadata of Chrome for Testing %s is invalid: it describes %s",
			version,
			*envelope.Version,
		)
	}
	var downloads struct {
		Chrome *[]json.RawMessage `json:"chrome"`
	}
	if err := json.Unmarshal(envelope.Downloads, &downloads); err != nil {
		return nil, err
	}
	if downloads.Chrome == nil {
		return nil, errors.New("chrome metadata has no downloads.chrome")
	}
	entries := make([]chromeDownload, 0, len(*downloads.Chrome))
	for _, raw := range *downloads.Chrome {
		var entry chromeDownload
		if err := json.Unmarshal(raw, &entry); err != nil {
			return nil, err
		}
		if entry.Platform == nil || entry.URL == nil {
			return nil, errors.New("chrome download requires platform and url")
		}
		entries = append(entries, entry)
	}

	return entries, nil
}

func measureChromeArchive(
	ctx context.Context,
	client *artifacts.Client,
	dir, location string,
	platform chromePlatform,
) (browserop.ReleasePlatform, artifacts.Digest, error) {
	path := filepath.Join(dir, platform.Name+".zip")
	file, err := os.Create(path)
	if err != nil {
		return browserop.ReleasePlatform{}, artifacts.Digest{}, err
	}
	digest, downloadErr := artifacts.DownloadMeasured(ctx, client, location, 1024*1024*1024, file)
	if err := errors.Join(downloadErr, file.Close()); err != nil {
		return browserop.ReleasePlatform{}, artifacts.Digest{}, err
	}
	holds, err := artifacts.ZipHolds(ctx, path, platform.Executable)
	if err != nil {
		return browserop.ReleasePlatform{}, artifacts.Digest{}, err
	}
	if !holds {
		return browserop.ReleasePlatform{}, artifacts.Digest{}, fmt.Errorf(
			"the %s archive holds no %s",
			platform.Name,
			platform.Executable,
		)
	}
	if err := os.Remove(path); err != nil {
		return browserop.ReleasePlatform{}, artifacts.Digest{}, err
	}
	entry := browserop.ReleasePlatform{
		Target:     platform.Target,
		URL:        location,
		Size:       digest.Size,
		SHA256:     digest.SHA256,
		Executable: platform.Executable,
	}
	return entry, digest, nil
}
