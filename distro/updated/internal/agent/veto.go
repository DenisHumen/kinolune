// SPDX-License-Identifier: MPL-2.0

package agent

import (
	"bytes"
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log/slog"
	"net/http"
	"os"
	"strings"
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
// (contract: distro/updated/README.md, "Вето медиасервера"; server: SRV-019).
type Activity struct {
	// Busy: do not reboot now (playback, paused sessions, transcoding, a
	// recording, a library scan — whatever the server knows about).
	Busy bool `json:"busy"`
	// Sessions is the number of active playback sessions; > 0 means busy.
	Sessions int `json:"sessions"`
	// Reason is shown in logs and the admin UI.
	Reason string `json:"reason,omitempty"`
}

// ErrPinMismatch: the server's certificate is not the one in its cert file.
var ErrPinMismatch = errors.New("media server certificate does not match the pinned fingerprint")

// HTTPVeto asks the media server on loopback.
//
// The media server speaks HTTPS with a self-signed certificate (SRV-004), so
// the agent pins it: at every call it reads CertFile, computes the SHA-256 of
// the DER certificate and accepts only a server presenting exactly that
// certificate (no system CAs, no blind InsecureSkipVerify).
//
//   - CertFile missing: the server is not installed or never started (it
//     creates the file on first start). Plain HTTP on the same address is
//     tried for a server configured with TLS off; if that does not answer
//     either, "not busy".
//   - Connection refused (server stopped) and 404 (a server without the
//     endpoint): "not busy" — nothing plays without a running server.
//   - The server answers with non-TLS bytes (TLS turned off, stale cert
//     file): retried over plain HTTP.
//   - Pin mismatch, other TLS errors, timeouts, other statuses, bad JSON:
//     an error, the agent postpones.
type HTTPVeto struct {
	// URL of the endpoint, normally https://127.0.0.1:8487/api/v1/system/activity.
	// An http:// URL disables TLS and pinning (server configured without TLS).
	URL string
	// CertFile is the server's certificate (PEM or DER), normally
	// /var/lib/alpha-media/tls/server.crt.
	CertFile string
	// Timeout per request (default 5s).
	Timeout time.Duration
	Logger  *slog.Logger
}

func (v HTTPVeto) log() *slog.Logger {
	if v.Logger == nil {
		return slog.New(slog.DiscardHandler)
	}
	return v.Logger
}

func (v HTTPVeto) timeout() time.Duration {
	if v.Timeout <= 0 {
		return 5 * time.Second
	}
	return v.Timeout
}

// plainURL turns https://… into http://… (same host, port and path).
func plainURL(u string) string {
	if rest, ok := strings.CutPrefix(u, "https://"); ok {
		return "http://" + rest
	}
	return u
}

// Busy implements Vetoer.
func (v HTTPVeto) Busy(ctx context.Context) (bool, string, error) {
	if !strings.HasPrefix(v.URL, "https://") {
		return v.ask(ctx, &http.Client{Timeout: v.timeout()}, v.URL)
	}
	pin, err := certFingerprint(v.CertFile)
	if errors.Is(err, fs.ErrNotExist) {
		busy, reason, perr := v.ask(ctx, &http.Client{Timeout: v.timeout()}, plainURL(v.URL))
		if perr != nil {
			v.log().Info("media server certificate not found and no plain-HTTP answer: treating the server as not installed",
				"cert", v.CertFile, "err", perr)
			return false, "", nil
		}
		return busy, reason, nil
	}
	if err != nil {
		return false, "", fmt.Errorf("media server certificate %s: %w", v.CertFile, err)
	}
	busy, reason, err := v.ask(ctx, pinnedClient(pin, v.timeout()), v.URL)
	var rh tls.RecordHeaderError
	if errors.As(err, &rh) || errors.Is(err, http.ErrSchemeMismatch) {
		v.log().Info("media server does not speak TLS, asking over plain HTTP", "url", plainURL(v.URL))
		return v.ask(ctx, &http.Client{Timeout: v.timeout()}, plainURL(v.URL))
	}
	if errors.Is(err, ErrPinMismatch) {
		v.log().Error("media server certificate pin mismatch: postponing the reboot",
			"cert", v.CertFile, "pinned", "sha256:"+hex.EncodeToString(pin[:]))
	}
	return busy, reason, err
}

// certFingerprint reads the first certificate of a PEM file (or a raw DER
// file) and returns the SHA-256 of its DER bytes.
func certFingerprint(path string) ([32]byte, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return [32]byte{}, err
	}
	der := data
	if bytes.Contains(data, []byte("-----BEGIN")) {
		der = nil
		for rest := data; ; {
			var b *pem.Block
			b, rest = pem.Decode(rest)
			if b == nil {
				break
			}
			if b.Type == "CERTIFICATE" {
				der = b.Bytes
				break
			}
		}
		if der == nil {
			return [32]byte{}, errors.New("no CERTIFICATE block")
		}
	}
	if len(der) == 0 {
		return [32]byte{}, errors.New("empty certificate file")
	}
	return sha256.Sum256(der), nil
}

// pinnedClient accepts only a server whose leaf certificate hashes to pin.
// Chain and host-name checks are replaced by the pin (self-signed server).
func pinnedClient(pin [32]byte, timeout time.Duration) *http.Client {
	tr := &http.Transport{
		TLSClientConfig: &tls.Config{
			MinVersion:         tls.VersionTLS12,
			InsecureSkipVerify: true, // verified by the pin below
			VerifyConnection: func(cs tls.ConnectionState) error {
				if len(cs.PeerCertificates) == 0 {
					return ErrPinMismatch
				}
				got := sha256.Sum256(cs.PeerCertificates[0].Raw)
				if subtle.ConstantTimeCompare(got[:], pin[:]) != 1 {
					return fmt.Errorf("%w (server presents sha256:%s)", ErrPinMismatch, hex.EncodeToString(got[:]))
				}
				return nil
			},
		},
		DisableKeepAlives: true,
		Proxy:             nil, // never through a proxy: loopback only
	}
	return &http.Client{Timeout: timeout, Transport: tr}
}

// ask does one GET and interprets the answer.
func (v HTTPVeto) ask(ctx context.Context, c *http.Client, url string) (bool, string, error) {
	if c.Transport == nil {
		c.Transport = &http.Transport{Proxy: nil, DisableKeepAlives: true}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
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
