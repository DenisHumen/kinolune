// SPDX-License-Identifier: MPL-2.0

package agent

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
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
	_, _, err := HTTPVeto{URL: srv.URL, Client: &http.Client{Timeout: 50 * time.Millisecond}}.Busy(context.Background())
	if err == nil {
		t.Fatal("a hanging server must be an error (postpone)")
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
