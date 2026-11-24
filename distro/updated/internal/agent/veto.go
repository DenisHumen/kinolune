// SPDX-License-Identifier: MPL-2.0

package agent

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"syscall"
	"time"
)

// Vetoer is the hook that stops an automatic reboot. The media server (or
// anything else that must not be cut off) answers "busy" while someone is
// watching or listening. An error means "do not know": the agent postpones.
// Explicit user actions ("apply now", "rollback now") are not vetoed.
type Vetoer interface {
	Busy(ctx context.Context) (busy bool, reason string, err error)
}

// NoVeto never objects.
type NoVeto struct{}

// Busy implements Vetoer.
func (NoVeto) Busy(context.Context) (bool, string, error) { return false, "", nil }

// MultiVeto is busy when any of its vetoers is busy or fails.
type MultiVeto []Vetoer

// Busy implements Vetoer.
func (m MultiVeto) Busy(ctx context.Context) (bool, string, error) {
	for _, v := range m {
		busy, reason, err := v.Busy(ctx)
		if err != nil || busy {
			return busy, reason, err
		}
	}
	return false, "", nil
}

// Activity is the JSON the media server returns from its activity endpoint
// (contract: distro/updated/README.md, "Вето медиасервера").
type Activity struct {
	// Busy: do not reboot now (playback, transcoding, a recording, a
	// library migration — whatever the server knows about).
	Busy bool `json:"busy"`
	// Sessions is the number of active playback sessions; > 0 means busy.
	Sessions int `json:"sessions"`
	// Reason is shown in logs and the admin UI.
	Reason string `json:"reason,omitempty"`
}

// HTTPVeto asks the media server over loopback HTTP.
//
// GET URL -> 200 + Activity JSON. Connection refused (server not running)
// and 404 (an older server without the endpoint) mean "not busy": nothing
// plays without a running server. Timeouts, other statuses and bad JSON are
// errors, so the agent postpones.
type HTTPVeto struct {
	URL    string
	Client *http.Client
}

// Busy implements Vetoer.
func (v HTTPVeto) Busy(ctx context.Context) (bool, string, error) {
	c := v.Client
	if c == nil {
		c = &http.Client{Timeout: 5 * time.Second}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, v.URL, nil)
	if err != nil {
		return false, "", err
	}
	req.Header.Set("Accept", "application/json")
	resp, err := c.Do(req)
	if err != nil {
		if errors.Is(err, syscall.ECONNREFUSED) {
			return false, "", nil
		}
		return false, "", err
	}
	defer resp.Body.Close()
	switch {
	case resp.StatusCode == http.StatusNotFound:
		return false, "", nil
	case resp.StatusCode != http.StatusOK:
		return false, "", fmt.Errorf("media activity: HTTP %d", resp.StatusCode)
	}
	var act Activity
	if err := json.NewDecoder(io.LimitReader(resp.Body, 64<<10)).Decode(&act); err != nil {
		return false, "", fmt.Errorf("media activity: %w", err)
	}
	if act.Busy || act.Sessions > 0 {
		reason := act.Reason
		if reason == "" {
			reason = fmt.Sprintf("%d active session(s)", act.Sessions)
		}
		return true, reason, nil
	}
	return false, "", nil
}

// RebootFunc adapts a function to Rebooter.
type RebootFunc func(ctx context.Context) error

// Reboot implements Rebooter.
func (f RebootFunc) Reboot(ctx context.Context) error { return f(ctx) }
