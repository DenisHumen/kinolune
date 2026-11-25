// SPDX-License-Identifier: MPL-2.0

// Package updproxy forwards the update API of the admin backend to
// alpha-updated's unix socket (/run/alpha-updated/api.sock, DIST-009). Only
// a fixed table of routes is forwarded, never an arbitrary path.
package updproxy

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"time"
)

// DefaultSocket is alpha-updated's API socket.
const DefaultSocket = "/run/alpha-updated/api.sock"

// MaxBody bounds request and response bodies.
const MaxBody = 1 << 20

// Route maps a public method+path to alpha-updated.
type Route struct {
	Method string
	// Path below the admin's update prefix ("" = the prefix itself).
	Path string
	// Upstream path on alpha-updated.
	Upstream string
	// Admin: changes something (admin role + CSRF needed).
	Admin bool
}

// Routes is the forwarded API (see distro/updated/README.md).
var Routes = []Route{
	{http.MethodGet, "", "/v1/update", false},
	{http.MethodGet, "/status", "/v1/status", false},
	{http.MethodPost, "/check", "/v1/update/check", true},
	{http.MethodPost, "/download", "/v1/update/download", true},
	{http.MethodPost, "/apply", "/v1/update/apply", true},
	{http.MethodDelete, "/apply", "/v1/update/apply", true},
	{http.MethodPost, "/rollback", "/v1/rollback", true},
	{http.MethodGet, "/settings", "/v1/settings", false},
	{http.MethodPut, "/settings", "/v1/settings", true},
}

// Find returns the route for method and the path below the prefix.
// ok is false for an unknown path; methodOK is false when the path exists
// with other methods.
func Find(method, sub string) (r Route, ok, pathKnown bool) {
	sub = strings.TrimSuffix(sub, "/")
	for _, rt := range Routes {
		if rt.Path != sub {
			continue
		}
		pathKnown = true
		if rt.Method == method {
			return rt, true, true
		}
	}
	return Route{}, false, pathKnown
}

// Client talks HTTP over the unix socket.
type Client struct {
	Socket string
	http   *http.Client
}

// New returns a client for socket (DefaultSocket when empty).
func New(socket string) *Client {
	if socket == "" {
		socket = DefaultSocket
	}
	c := &Client{Socket: socket}
	c.http = &http.Client{
		Timeout: 60 * time.Second,
		Transport: &http.Transport{
			DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
				var d net.Dialer
				return d.DialContext(ctx, "unix", c.Socket)
			},
			MaxIdleConns:    2,
			IdleConnTimeout: 30 * time.Second,
		},
	}
	return c
}

// Response is alpha-updated's answer.
type Response struct {
	Status      int
	ContentType string
	Body        []byte
}

// Do forwards one request. body may be nil.
func (c *Client) Do(ctx context.Context, r Route, body io.Reader) (*Response, error) {
	req, err := http.NewRequestWithContext(ctx, r.Method, "http://alpha-updated"+r.Upstream, body)
	if err != nil {
		return nil, err
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, fmt.Errorf("alpha-updated: %w", err)
	}
	defer resp.Body.Close()
	b, err := io.ReadAll(io.LimitReader(resp.Body, MaxBody+1))
	if err != nil {
		return nil, fmt.Errorf("alpha-updated: %w", err)
	}
	if len(b) > MaxBody {
		return nil, fmt.Errorf("alpha-updated: answer too large")
	}
	ct := resp.Header.Get("Content-Type")
	if ct == "" {
		ct = "application/json"
	}
	return &Response{Status: resp.StatusCode, ContentType: ct, Body: b}, nil
}
