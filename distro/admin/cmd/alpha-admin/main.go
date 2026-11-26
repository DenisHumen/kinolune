// SPDX-License-Identifier: MPL-2.0

// Command alpha-admin is the admin backend of the NAS OS (DIST-013): login
// with OS users, REST over the system D-Bus services and a proxy to the
// update agent, over HTTPS with a self-signed certificate, LAN only by
// default. See distro/admin/README.md.
package main

import (
	"context"
	"crypto/tls"
	"errors"
	"flag"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/auth"
	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/netpolicy"
	"github.com/DenisHumen/kinolune/distro/admin/internal/privhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/server"
	"github.com/DenisHumen/kinolune/distro/admin/internal/system"
	"github.com/DenisHumen/kinolune/distro/admin/internal/tlscert"
	"github.com/DenisHumen/kinolune/distro/admin/internal/updproxy"
)

type options struct {
	listen        string
	httpListen    string
	stateDir      string
	authSocket    string
	privSocket    string
	updatedSocket string
	installInfo   string
	versionFile   string
	adminGroups   string
	allowPublic   bool
	journalctl    string
	logLevel      string
}

func main() {
	var o options
	flag.StringVar(&o.listen, "listen", ":443", "HTTPS address")
	flag.StringVar(&o.httpListen, "http-listen", ":80", "plain HTTP address that only redirects to HTTPS and answers /health (empty: off)")
	flag.StringVar(&o.stateDir, "state-dir", "/var/lib/alpha/admin", "state directory (TLS certificate)")
	flag.StringVar(&o.authSocket, "auth-socket", "/run/alpha-admin/auth.sock", "socket of the password helper (alpha-admin-auth)")
	flag.StringVar(&o.privSocket, "priv-socket", "/run/alpha-admin/priv.sock", "socket of the users/Samba/SSH helper (alpha-admin-priv)")
	flag.StringVar(&o.updatedSocket, "updated-socket", updproxy.DefaultSocket, "API socket of alpha-updated")
	flag.StringVar(&o.installInfo, "install-info", server.InstallInfoPath, "install.json written by the installer")
	flag.StringVar(&o.versionFile, "version-file", "/usr/share/alpha/version", "OS version file")
	flag.StringVar(&o.adminGroups, "admin-groups", strings.Join(auth.DefaultAdminGroups, ","), "groups whose members are administrators")
	flag.BoolVar(&o.allowPublic, "allow-public", false, "also accept clients from public addresses (not only LAN)")
	flag.StringVar(&o.journalctl, "journalctl", "journalctl", "journalctl binary")
	flag.StringVar(&o.logLevel, "log-level", "info", "debug, info, warn, error")
	flag.Parse()

	var level slog.Level
	if err := level.UnmarshalText([]byte(o.logLevel)); err != nil {
		fmt.Fprintln(os.Stderr, "alpha-admin: bad -log-level:", err)
		os.Exit(2)
	}
	log := slog.New(slog.NewJSONHandler(os.Stderr, &slog.HandlerOptions{Level: level}))
	if err := run(o, log); err != nil {
		log.Error("alpha-admin stopped", "err", err)
		os.Exit(1)
	}
}

func run(o options, log *slog.Logger) error {
	hostname, _ := os.Hostname()
	cert, err := tlscert.LoadOrCreate(o.stateDir, hostname, time.Now())
	if err != nil {
		return err
	}
	if cert.Generated {
		log.Info("generated TLS certificate", "fingerprint", cert.Fingerprint)
	}
	install, err := server.LoadInstallInfo(o.installInfo)
	if err != nil {
		log.Warn("install.json unreadable, continuing without it", "err", err)
	}
	bus, err := system.SystemBus()
	if err != nil {
		return fmt.Errorf("system bus: %w", err)
	}
	defer bus.Close()

	srv := &server.Server{
		Verifier:    &authhelper.Client{Socket: o.authSocket, Timeout: 15 * time.Second},
		Sessions:    auth.NewStore(),
		Limiter:     auth.NewLimiter(auth.DefaultLimiterConfig, nil),
		AdminGroups: splitList(o.adminGroups),
		Services:    &system.Systemd{Bus: bus, Journalctl: o.journalctl},
		Network:     &system.NetworkManager{Bus: bus},
		Firewall:    &system.Firewalld{Bus: bus},
		Disks:       &system.UDisks{Bus: bus, SystemUUIDs: install.SystemUUIDs()},
		Power:       &system.Logind{Bus: bus},
		Updates:     updproxy.New(o.updatedSocket),
		Accounts:    &privhelper.Client{Socket: o.privSocket},
		AdminNow: func(user string) bool {
			return auth.DefaultGroupFiles.IsAdminNow(user, splitList(o.adminGroups))
		},
		Info: server.Info{
			OSVersion:      server.ReadOSVersion(o.versionFile),
			TLSFingerprint: cert.Fingerprint,
			Install:        install,
		},
		Log: log,
	}

	rejected := func(a net.Addr) { log.Warn("refused non-LAN client", "addr", a.String()) }
	ln, err := net.Listen("tcp", o.listen)
	if err != nil {
		return err
	}
	tlsLn := tls.NewListener(&netpolicy.Listener{Listener: ln, AllowPublic: o.allowPublic, Rejected: rejected}, &tls.Config{
		Certificates: []tls.Certificate{cert.TLS},
		MinVersion:   tls.VersionTLS12,
		NextProtos:   []string{"h2", "http/1.1"},
	})
	httpsSrv := &http.Server{
		Handler:           srv,
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      90 * time.Second,
		IdleTimeout:       120 * time.Second,
		MaxHeaderBytes:    64 << 10,
		ErrorLog:          slog.NewLogLogger(log.Handler(), slog.LevelDebug),
	}
	errs := make(chan error, 2)
	go func() { errs <- httpsSrv.Serve(tlsLn) }()
	log.Info("alpha-admin listening", "https", o.listen, "http", o.httpListen, "lan_only", !o.allowPublic,
		"fingerprint", cert.Fingerprint, "pam_helper", o.authSocket)

	var httpSrv *http.Server
	if o.httpListen != "" {
		hl, err := net.Listen("tcp", o.httpListen)
		if err != nil {
			return err
		}
		httpSrv = &http.Server{
			Handler:           redirect(o.listen),
			ReadHeaderTimeout: 10 * time.Second,
			ReadTimeout:       10 * time.Second,
			WriteTimeout:      10 * time.Second,
			MaxHeaderBytes:    16 << 10,
			ErrorLog:          slog.NewLogLogger(log.Handler(), slog.LevelDebug),
		}
		go func() {
			errs <- httpSrv.Serve(&netpolicy.Listener{Listener: hl, AllowPublic: o.allowPublic, Rejected: rejected})
		}()
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer stop()
	select {
	case err := <-errs:
		if !errors.Is(err, http.ErrServerClosed) {
			return err
		}
	case <-ctx.Done():
		log.Info("stopping")
	}
	shutdown, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if httpSrv != nil {
		_ = httpSrv.Shutdown(shutdown)
	}
	return httpsSrv.Shutdown(shutdown)
}

// redirect answers /health and /healthz on plain HTTP (greenboot and VM
// checks) and sends everything else to HTTPS on the same host.
func redirect(httpsAddr string) http.Handler {
	_, port, _ := net.SplitHostPort(httpsAddr)
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/health" || r.URL.Path == "/healthz" {
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"service":"alpha-admin","status":"ok"}` + "\n"))
			return
		}
		host := r.Host
		if h, _, err := net.SplitHostPort(host); err == nil {
			host = h
		}
		if strings.Contains(host, ":") {
			host = "[" + host + "]"
		}
		if host == "" {
			host = "localhost"
		}
		if port != "" && port != "443" {
			host += ":" + port
		}
		target := "https://" + host + r.URL.RequestURI()
		http.Redirect(w, r, target, http.StatusMovedPermanently)
	})
}

func splitList(s string) []string {
	var out []string
	for _, p := range strings.Split(s, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}
