// SPDX-License-Identifier: MPL-2.0

package agent

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"errors"
	"log/slog"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestHTTPVeto(t *testing.T) {
	cases := []struct {
		name    string
		status  int
		body    string
		busy    bool
		wantErr bool
	}{
		{"idle", 200, `{"busy":false,"sessions":0}`, false, false},
		{"sessions", 200, `{"sessions":2}`, true, false},
		{"busy flag", 200, `{"busy":true,"reason":"transcoding"}`, true, false},
		{"old server", 404, `not found`, false, false},
		{"server error", 503, ``, false, true},
		{"bad json", 200, `{`, false, true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.WriteHeader(tc.status)
				w.Write([]byte(tc.body))
			}))
			defer srv.Close()
			busy, reason, err := HTTPVeto{URL: srv.URL}.Busy(context.Background())
			if busy != tc.busy || (err != nil) != tc.wantErr {
				t.Fatalf("busy=%v reason=%q err=%v", busy, reason, err)
			}
			if busy && reason == "" {
				t.Error("busy without a reason")
			}
		})
	}
}

func TestHTTPVetoServerNotRunning(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := ln.Addr().String()
	ln.Close() // nothing listens there now
	busy, _, err := HTTPVeto{URL: "http://" + addr + "/api/v1/system/activity"}.Busy(context.Background())
	if busy || err != nil {
		t.Fatalf("not running: busy=%v err=%v", busy, err)
	}
}

func TestHTTPVetoTimeout(t *testing.T) {
	release := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		<-release
	}))
	defer srv.Close()
	defer close(release)
	_, _, err := HTTPVeto{URL: srv.URL, Timeout: 50 * time.Millisecond}.Busy(context.Background())
	if err == nil {
		t.Fatal("a hanging server must be an error (postpone)")
	}
}

// writeCert stores a certificate as the server would (PEM).
func writeCert(t *testing.T, der []byte) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "server.crt")
	if err := os.WriteFile(p, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

// otherCert makes a self-signed ECDSA P-256 certificate (like the media
// server's) that no test server presents.
func otherCert(t *testing.T) []byte {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	tmpl := &x509.Certificate{
		SerialNumber: big.NewInt(1),
		Subject:      pkix.Name{CommonName: "alpha-media"},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(time.Hour),
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	return der
}

func activityServer(tls bool, body string) *httptest.Server {
	h := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/v1/system/activity" {
			http.NotFound(w, r)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(body))
	})
	if tls {
		return httptest.NewTLSServer(h)
	}
	return httptest.NewServer(h)
}

func httpsURL(srv *httptest.Server) string {
	return "https://" + strings.TrimPrefix(strings.TrimPrefix(srv.URL, "https://"), "http://") + "/api/v1/system/activity"
}

func TestHTTPVetoTLSPinned(t *testing.T) {
	srv := activityServer(true, `{"busy":true,"sessions":1,"reason":"paused: Dune"}`)
	defer srv.Close()
	v := HTTPVeto{URL: httpsURL(srv), CertFile: writeCert(t, srv.Certificate().Raw)}
	busy, reason, err := v.Busy(context.Background())
	if !busy || reason != "paused: Dune" || err != nil {
		t.Fatalf("pinned: busy=%v reason=%q err=%v", busy, reason, err)
	}
	// DER file works too.
	der := filepath.Join(t.TempDir(), "server.der")
	os.WriteFile(der, srv.Certificate().Raw, 0o644)
	if busy, _, err := (HTTPVeto{URL: v.URL, CertFile: der}).Busy(context.Background()); !busy || err != nil {
		t.Fatalf("der: %v %v", busy, err)
	}
}

func TestHTTPVetoTLSPinMismatch(t *testing.T) {
	srv := activityServer(true, `{"busy":false}`)
	defer srv.Close()
	// httptest servers share one built-in certificate: pin a freshly made one.
	v := HTTPVeto{URL: httpsURL(srv), CertFile: writeCert(t, otherCert(t))}
	busy, _, err := v.Busy(context.Background())
	if busy || !errors.Is(err, ErrPinMismatch) {
		t.Fatalf("mismatch must be an error (postpone): busy=%v err=%v", busy, err)
	}
	// And the agent postpones on it.
	a := &Agent{veto: v, log: slog.New(slog.DiscardHandler)}
	if vetoed, _ := a.vetoed(context.Background()); !vetoed {
		t.Fatal("pin mismatch did not postpone")
	}
}

func TestHTTPVetoCertMissing(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "tls", "server.crt")
	// Nothing listens: server not installed.
	ln, _ := net.Listen("tcp", "127.0.0.1:0")
	addr := ln.Addr().String()
	ln.Close()
	v := HTTPVeto{URL: "https://" + addr + "/api/v1/system/activity", CertFile: missing}
	if busy, _, err := v.Busy(context.Background()); busy || err != nil {
		t.Fatalf("not installed: busy=%v err=%v", busy, err)
	}
	// A server configured without TLS (no cert file) is asked over plain HTTP.
	plain := activityServer(false, `{"sessions":2}`)
	defer plain.Close()
	v.URL = httpsURL(plain)
	if busy, _, err := v.Busy(context.Background()); !busy || err != nil {
		t.Fatalf("plain fallback without cert: busy=%v err=%v", busy, err)
	}
}

func TestHTTPVetoStaleCertPlainServer(t *testing.T) {
	// The cert file is still there but the server now runs with TLS off: it
	// answers the TLS hello with an HTTP 400, the agent retries over HTTP.
	plain := activityServer(false, `{"busy":true,"reason":"recording"}`)
	defer plain.Close()
	v := HTTPVeto{URL: httpsURL(plain), CertFile: writeCert(t, otherCert(t))}
	if busy, reason, err := v.Busy(context.Background()); !busy || reason != "recording" || err != nil {
		t.Fatalf("stale cert, plain server: busy=%v reason=%q err=%v", busy, reason, err)
	}
}

func TestHTTPVetoBadCertFile(t *testing.T) {
	p := filepath.Join(t.TempDir(), "server.crt")
	os.WriteFile(p, []byte("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n"), 0o644)
	srv := activityServer(true, `{}`)
	defer srv.Close()
	if _, _, err := (HTTPVeto{URL: httpsURL(srv), CertFile: p}).Busy(context.Background()); err == nil {
		t.Fatal("a cert file without a certificate must be an error")
	}
}

func TestHTTPVetoTLSTimeout(t *testing.T) {
	release := make(chan struct{})
	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { <-release }))
	defer srv.Close()
	defer close(release)
	v := HTTPVeto{URL: httpsURL(srv), CertFile: writeCert(t, srv.Certificate().Raw), Timeout: 50 * time.Millisecond}
	if _, _, err := v.Busy(context.Background()); err == nil {
		t.Fatal("a hanging HTTPS server must be an error (postpone)")
	}
}

type staticVeto struct {
	busy bool
	err  error
}

func (s staticVeto) Busy(context.Context) (bool, string, error) { return s.busy, "x", s.err }

func TestMultiVeto(t *testing.T) {
	if busy, _, _ := (MultiVeto{staticVeto{}, staticVeto{}}).Busy(context.Background()); busy {
		t.Error("all idle must be idle")
	}
	if busy, _, _ := (MultiVeto{staticVeto{}, staticVeto{busy: true}}).Busy(context.Background()); !busy {
		t.Error("one busy must be busy")
	}
	if _, _, err := (MultiVeto{staticVeto{err: errors.New("x")}}).Busy(context.Background()); err == nil {
		t.Error("error must pass through")
	}
}
