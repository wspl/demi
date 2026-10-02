// Package tabs owns Chrome launch, capture connections, environment and tab
// lifetimes, and the registry and state shared by page actions and the live view.
// It knows no conversations or viewers. Owners explicitly close and join their
// resources; command checkouts release their tab gate without waiting.
package tabs
