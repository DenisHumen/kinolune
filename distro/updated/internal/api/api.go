// SPDX-License-Identifier: MPL-2.0

// Package api serves the alpha-updated JSON API on a unix socket.
//
// Access control is the socket itself: /run/alpha-updated/api.sock is
// root:alpha-admin 0660 (systemd socket unit), so only root and the admin
// backend reach it. Every mutating request is logged with the peer's
// uid/gid/pid (SO_PEERCRED).
package api

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"time"

	"github.com/DenisHumen/kinolune/distro/updated/internal/agent"
	"github.com/DenisHumen/kinolune/distro/updated/internal/settings"
)

// Agent is what the API needs from the agent.
type Agent interface {
	Status(ctx context.Context) agent.Status
	Check() (agent.Operation, error)
	Download() (agent.Operation, error)
	Apply(when string) (*agent.Operation, error)
	CancelPending()
	Rollback(reboot bool) (agent.Operation, error)
	Settings() settings.Settings
	SetSettings(p settings.Patch) (settings.Settings, *agent.Operation, error)
}

// Server is the HTTP handler.
type Server struct {
	A   Agent
	Log *slog.Logger
}

const maxBody = 16 << 10

// Handler returns the routes.
//
//	GET  /healthz
//	GET  /v1/status
//	GET  /v1/update
//	POST /v1/update/check
//	POST /v1/update/download
//	POST /v1/update/apply      {"when": "now" | "maintenance"}
//	DELETE /v1/update/apply    (cancel a "maintenance" request)
//	POST /v1/rollback          {"reboot": true}
//	GET  /v1/settings
//	PUT  /v1/settings          {"channel", "auto", "window"} (any subset)
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, map[string]string{"service": "alpha-updated", "status": "ok"})
	})
	mux.HandleFunc("GET /v1/status", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, s.A.Status(r.Context()))
	})
	mux.HandleFunc("GET /v1/update", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, s.A.Status(r.Context()).Update)
	})
	mux.HandleFunc("POST /v1/update/check", s.audited(func(w http.ResponseWriter, r *http.Request) {
		op, err := s.A.Check()
		s.started(w, &op, err)
	}))
	mux.HandleFunc("POST /v1/update/download", s.audited(func(w http.ResponseWriter, r *http.Request) {
		op, err := s.A.Download()
		s.started(w, &op, err)
	}))
	mux.HandleFunc("POST /v1/update/apply", s.audited(func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			When string `json:"when"`
		}
		if !decode(w, r, &body, true) {
			return
		}
		op, err := s.A.Apply(body.When)
		if err == nil && op == nil {
			writeJSON(w, http.StatusAccepted, map[string]any{"pending_apply": true})
			return
		}
		s.started(w, op, err)
	}))
	mux.HandleFunc("DELETE /v1/update/apply", s.audited(func(w http.ResponseWriter, r *http.Request) {
		s.A.CancelPending()
		writeJSON(w, http.StatusOK, map[string]any{"pending_apply": false})
	}))
	mux.HandleFunc("POST /v1/rollback", s.audited(func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			Reboot bool `json:"reboot"`
		}
		if !decode(w, r, &body, true) {
			return
		}
		op, err := s.A.Rollback(body.Reboot)
		s.started(w, &op, err)
	}))
	mux.HandleFunc("GET /v1/settings", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, s.A.Settings())
	})
	mux.HandleFunc("PUT /v1/settings", s.audited(func(w http.ResponseWriter, r *http.Request) {
		var p settings.Patch
		if !decode(w, r, &p, false) {
			return
		}
		next, op, err := s.A.SetSettings(p)
		if err != nil {
			writeError(w, err)
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"settings": next, "operation": op})
	}))
	return mux
}

// started answers 202 with the operation or maps the error.
func (s *Server) started(w http.ResponseWriter, op *agent.Operation, err error) {
	if err != nil {
		if errors.Is(err, agent.ErrBusy) {
			writeJSON(w, http.StatusConflict, map[string]any{"error": err.Error(), "code": "busy", "operation": op})
			return
		}
		writeError(w, err)
		return
	}
	writeJSON(w, http.StatusAccepted, map[string]any{"operation": op})
}

func writeError(w http.ResponseWriter, err error) {
	code, status := "internal", http.StatusInternalServerError
	switch {
	case errors.Is(err, agent.ErrBusy):
		code, status = "busy", http.StatusConflict
	case errors.Is(err, agent.ErrInvalid):
		code, status = "bad_request", http.StatusBadRequest
	case errors.Is(err, agent.ErrNoUpdate), errors.Is(err, agent.ErrNoRollback):
		code, status = "conflict", http.StatusConflict
	}
	writeJSON(w, status, map[string]string{"error": err.Error(), "code": code})
}

// decode reads a small JSON body; an empty body is allowed when optional.
func decode(w http.ResponseWriter, r *http.Request, v any, optional bool) bool {
	r.Body = http.MaxBytesReader(w, r.Body, maxBody)
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		if optional && errors.Is(err, io.EOF) {
			return true
		}
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": "bad JSON: " + err.Error(), "code": "bad_request"})
		return false
	}
	return true
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

// audited logs who asked for a change.
func (s *Server) audited(h http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		attrs := []any{"method", r.Method, "path", r.URL.Path}
		if c, ok := r.Context().Value(credKey{}).(Cred); ok {
			attrs = append(attrs, "uid", c.UID, "gid", c.GID, "pid", c.PID)
		}
		if s.Log != nil {
			s.Log.Info("api request", attrs...)
		}
		h(w, r)
	}
}

// Cred is the peer of a unix-socket connection.
type Cred struct {
	UID, GID uint32
	PID      int32
}

type credKey struct{}

// NewHTTPServer wires the handler with peer credentials and timeouts.
func (s *Server) NewHTTPServer() *http.Server {
	return &http.Server{
		Handler:           s.Handler(),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      60 * time.Second,
		ConnContext: func(ctx context.Context, c net.Conn) context.Context {
			if uc, ok := c.(*net.UnixConn); ok {
				if cred, ok := peerCred(uc); ok {
					return context.WithValue(ctx, credKey{}, cred)
				}
			}
			return ctx
		},
	}
}
