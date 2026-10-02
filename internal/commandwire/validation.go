package commandwire

import (
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"net/url"
	"path/filepath"
	"regexp"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/contract"
)

// validateInvocation checks process inputs that require domain rules.
func validateInvocation(v Invocation) error { return validateProcessInputs(v.Args, v.Cwd, v.Env) }
func validateLocalInvocation(v LocalInvocation) error {
	return validateProcessInputs(v.Args, v.Cwd, v.Env)
}

// validateProcessInputs prevents malformed command arguments and OS inputs.
func validateProcessInputs(args json.RawMessage, cwd string, env map[string]string) error {
	if _, err := contract.Object(args); err != nil {
		return contract.At("args", err)
	}
	if strings.ContainsRune(cwd, 0) {
		return errors.New("cwd contains NUL")
	}
	for name, value := range env {
		if name == "" || strings.ContainsAny(name, "\x00=") {
			return fmt.Errorf("invalid environment variable name %q", name)
		}
		if strings.ContainsRune(value, 0) {
			return fmt.Errorf("environment variable %q contains NUL", name)
		}
	}
	return nil
}

func validateConversationStatus(v ConversationStatus) error {
	for _, name := range v.Conversations {
		if name == "" {
			return errors.New("conversation must not be empty")
		}
	}
	return nil
}
func validateNumbersAnswer(v NumbersAnswer) error {
	if (v.First == nil) == (v.Error == nil) {
		return errors.New("numbers answer requires exactly one of first or error")
	}
	return nil
}
func validateArtifactRequest(v ArtifactRequest) error {
	if (v.Install == nil) == (v.Installed == nil) {
		return errors.New("artifact request requires exactly one of install or installed")
	}
	return nil
}
func validateArtifactAnswer(v ArtifactAnswer) error {
	n := 0
	if v.Path != nil {
		n++
	}
	if v.Installed != nil {
		n++
	}
	if v.Error != nil {
		n++
	}
	if n != 1 {
		return errors.New("artifact answer requires exactly one of path, installed or error")
	}
	return nil
}
func validateEditContext(v EditContext) error {
	for _, path := range []string{v.Directory, v.Lock} {
		if strings.ContainsRune(path, 0) || !filepath.IsAbs(path) {
			return errors.New("edit context requires absolute paths without NUL")
		}
	}
	return nil
}
func validateEditCopies(v EditCopies) error {
	for _, path := range []*string{v.Original, v.Modified} {
		if path != nil && strings.ContainsRune(*path, 0) {
			return errors.New("edit copy path contains NUL")
		}
	}
	return nil
}
func validateEditFile(v EditFile) error {
	if strings.ContainsRune(v.Path, 0) {
		return errors.New("edit file path contains NUL")
	}
	return nil
}

// validateArchiveEntry checks the slash-separated path inside an artifact archive.
func validateArchiveEntry(entry string) error {
	if strings.Contains(entry, "\\") {
		return errors.New("archive entry contains a backslash")
	}
	for _, part := range strings.Split(entry, "/") {
		if part == "" || part == "." || part == ".." {
			return errors.New("archive entry must have normal relative components")
		}
	}
	return nil
}
func validateResourceArtifact(v ResourceArtifact) error { return validateArchiveEntry(v.Entry) }
func validateArtifactArchive(v ArtifactArchive) error   { return validateArchiveEntry(v.Entry) }
func validatePackageResource(v PackageResource) error {
	if len(v.Targets) == 0 {
		return errors.New("resource requires at least one target")
	}
	for target := range v.Targets {
		if err := TargetTriple(target).Validate(); err != nil {
			return contract.At("targets."+target, err)
		}
	}
	return nil
}

var resourceName = regexp.MustCompile(`^[a-z][a-z0-9-]{0,63}$`)

func validatePackageDescriptor(v PackageDescriptor) error {
	if err := validateOperations(v.Operations); err != nil {
		return err
	}
	for target := range v.Targets {
		if err := TargetTriple(target).Validate(); err != nil {
			return contract.At("targets."+target, err)
		}
	}
	for name := range v.Resources {
		if !resourceName.MatchString(name) {
			return fmt.Errorf("invalid resource name %q", name)
		}
	}
	return nil
}
func validateServiceInfo(v ServiceInfo) error { return validateOperations(v.Operations) }

// validateOperations checks a service catalog for empty or repeated operation IDs.
func validateOperations(operations []string) error {
	seen := make(map[string]bool, len(operations))
	for _, op := range operations {
		if op == "" || seen[op] {
			return fmt.Errorf("empty or duplicate operation %q", op)
		}
		seen[op] = true
	}
	return nil
}
func validateArtifactURL(v ArtifactURL) error {
	canonical, err := contract.HTTPURL(v.URL)
	if err != nil {
		return err
	}
	parsed, err := url.Parse(canonical)
	if err != nil {
		return fmt.Errorf("artifact URL: %w", err)
	}
	if parsed.User != nil && (parsed.User.Username() != "" || strings.Contains(parsed.User.String(), ":")) {
		return errors.New("artifact URL must not contain credentials")
	}
	return nil
}

// IsDigest reports whether value is a SHA-256 digest in lowercase hexadecimal.
func IsDigest(value string) bool {
	return len(value) == 64 && strings.Trim(value, "0123456789abcdef") == ""
}

// Digest validates an artifact or descriptor's SHA-256 identity.
func Digest(value string) error {
	if !IsDigest(value) {
		return errors.New("is not a SHA-256 digest")
	}
	return nil
}

// IsTarget reports whether target is one of the published Targets.
func IsTarget(target string) bool {
	return slices.Contains(Targets, target)
}

// validateTargetTriple checks the command package's supported platform catalog.
func validateTargetTriple(target TargetTriple) error {
	if !IsTarget(string(target)) {
		return fmt.Errorf("unknown target %s", target)
	}
	return nil
}

// TargetArtifacts validates the targets and executable artifacts of a release.
// An empty map is valid for a development release with no artifacts.
func TargetArtifacts(targets map[string]PackageArtifact) error {
	for _, target := range slices.Sorted(maps.Keys(targets)) {
		if err := validateTargetTriple(TargetTriple(target)); err != nil {
			return contract.At(target, err)
		}
		if err := targets[target].Validate(); err != nil {
			return contract.At(target, err)
		}
	}
	return nil
}
