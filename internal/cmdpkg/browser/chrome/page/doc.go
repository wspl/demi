// Package page implements observations and actions over tabs owned by package tabs.
// Actions acquire one tab checkout, share the caller's deadline with the tab
// lifetime, and return operation errors preserving input progress and causes.
// CommandAdmitted and ScreenshotBytes instead use the caller's existing checkout.
// No page function owns a tab, environment, or CDP session.
package page
