// SPDX-License-Identifier: MPL-2.0

// Command alpha-updated is the update agent of the NAS OS: it checks,
// downloads and applies OS images with bootc, rolls back, and reboots only
// in the maintenance window when nobody is watching. API: JSON over the unix
// socket /run/alpha-updated/api.sock. See distro/updated/README.md.
package main

import (
	"bufio"
	"context"
	"errors"
	"flag"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/exec"
	"os/signal"
	"os/user"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/DenisHumen/kinolune/distro/updated/internal/agent"
	"github.com/DenisHumen/kinolune/distro/updated/internal/api"
	"github.com/DenisHumen/kinolune/distro/updated/internal/bootc"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "alpha-updated:", err)
		os.Exit(1)
	}
}

func run() error {
	stateDir := os.Getenv("STATE_DIRECTORY") // set by systemd StateDirectory=
	if stateDir == "" {
		stateDir = "/var/lib/alpha/updated"
	}
	var (
		socketPath  = flag.String("socket", "/run/alpha-updated/api.sock", "unix socket when not socket-activated")
		socketGroup = flag.String("socket-group", "alpha-admin", "group that may use the socket (when not socket-activated)")
		state       = flag.String("state-dir", stateDir, "settings and state directory")
		bootcPath   = flag.String("bootc", "/usr/bin/bootc", "bootc binary")
		versionFile = flag.String("version-file", "/usr/share/alpha/version", "image version file")
		mediaURL    = flag.String("media-activity-url", envOr("ALPHA_MEDIA_ACTIVITY_URL", "https://127.0.0.1:8487/api/v1/system/activity"),
			"media server activity endpoint that can veto a reboot; empty disables the veto; http:// only for a server without TLS")
		mediaCert = flag.String("media-cert", envOr("ALPHA_MEDIA_TLS_CERT", "/var/lib/alpha-media/tls/server.crt"),
			"media server certificate whose SHA-256 is pinned for the activity request")
		rebootCmd = flag.String("reboot-command", "systemctl reboot", "command that reboots the machine")
		interval  = flag.Duration("check-interval", 6*time.Hour, "time between automatic checks")
		tick      = flag.Duration("tick", time.Minute, "scheduler step")
		delay     = flag.Duration("start-delay", 2*time.Minute, "wait before the first scheduler step (network, media server)")
		logLevel  = flag.String("log-level", "info", "debug, info, warn or error")
	)
	flag.Parse()

	var lvl slog.Level
	if err := lvl.UnmarshalText([]byte(*logLevel)); err != nil {
		return err
	}
	log := slog.New(slog.NewJSONHandler(os.Stderr, &slog.HandlerOptions{Level: lvl}))

	ver := readVersion(*versionFile)
	var veto agent.Vetoer = agent.NoVeto{}
	if *mediaURL != "" {
		veto = agent.HTTPVeto{URL: *mediaURL, CertFile: *mediaCert, Logger: log}
	}
	rebootArgs := strings.Fields(*rebootCmd)
	if len(rebootArgs) == 0 {
		return errors.New("-reboot-command is empty")
	}
	rebooter := agent.RebootFunc(func(ctx context.Context) error {
		out, err := exec.CommandContext(ctx, rebootArgs[0], rebootArgs[1:]...).CombinedOutput()
		if err != nil {
			return fmt.Errorf("%s: %w: %s", *rebootCmd, err, strings.TrimSpace(string(out)))
		}
		return nil
	})

	a := agent.New(agent.Config{
		StateDir:      *state,
		CheckInterval: *interval,
		DefaultRepo:   ver["ALPHA_IMAGE_REPO"],
		OSVersion:     ver["ALPHA_OS_VERSION"],
		Logger:        log,
	}, bootc.Client{R: bootc.ExecRunner{Path: *bootcPath}}, veto, rebooter)

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	ln, activated, err := listen(*socketPath, *socketGroup)
	if err != nil {
		return err
	}
	log.Info("alpha-updated started", "socket", ln.Addr().String(), "socket_activated", activated,
		"state_dir", *state, "os_version", ver["ALPHA_OS_VERSION"], "media_veto", *mediaURL != "")

	a.Start(ctx)
	srv := (&api.Server{A: a, Log: log}).NewHTTPServer()
	errc := make(chan error, 1)
	go func() { errc <- srv.Serve(ln) }()
	go a.Run(ctx, *delay, *tick)

	select {
	case <-ctx.Done():
	case err := <-errc:
		if !errors.Is(err, http.ErrServerClosed) {
			return err
		}
	}
	log.Info("shutting down")
	sctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	_ = srv.Shutdown(sctx)
	a.Wait()
	return nil
}

func envOr(k, def string) string {
	if v, ok := os.LookupEnv(k); ok {
		return v
	}
	return def
}

// listen takes the socket from systemd (LISTEN_FDS) or creates it.
func listen(path, group string) (net.Listener, bool, error) {
	if pid, _ := strconv.Atoi(os.Getenv("LISTEN_PID")); pid == os.Getpid() {
		if n, _ := strconv.Atoi(os.Getenv("LISTEN_FDS")); n >= 1 {
			os.Unsetenv("LISTEN_PID")
			os.Unsetenv("LISTEN_FDS")
			os.Unsetenv("LISTEN_FDNAMES")
			f := os.NewFile(3, "alpha-updated.socket")
			ln, err := net.FileListener(f)
			f.Close()
			if err != nil {
				return nil, false, fmt.Errorf("socket activation: %w", err)
			}
			return ln, true, nil
		}
	}
	if err := os.MkdirAll(dirOf(path), 0o755); err != nil {
		return nil, false, err
	}
	_ = os.Remove(path) // stale socket of a previous run
	ln, err := net.Listen("unix", path)
	if err != nil {
		return nil, false, err
	}
	if err := os.Chmod(path, 0o660); err != nil {
		ln.Close()
		return nil, false, err
	}
	if group != "" {
		if g, err := user.LookupGroup(group); err == nil {
			if gid, err := strconv.Atoi(g.Gid); err == nil {
				_ = os.Chown(path, -1, gid)
			}
		}
	}
	return ln, false, nil
}

func dirOf(p string) string {
	if i := strings.LastIndex(p, "/"); i > 0 {
		return p[:i]
	}
	return "."
}

// readVersion parses KEY=VALUE lines of /usr/share/alpha/version.
func readVersion(path string) map[string]string {
	m := map[string]string{}
	f, err := os.Open(path)
	if err != nil {
		return m
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		if k, v, ok := strings.Cut(strings.TrimSpace(sc.Text()), "="); ok {
			m[k] = v
		}
	}
	return m
}
