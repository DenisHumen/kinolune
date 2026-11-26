// SPDX-License-Identifier: MPL-2.0

// Package fakebootc is an in-memory bootc for tests: it implements
// bootc.Runner, keeps booted/staged/rollback deployments and a registry, and
// answers `bootc status --format=json` in the real JSON shape.
package fakebootc

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"sync"

	"github.com/DenisHumen/kinolune/distro/updated/internal/bootc"
)

// Image is one image in the fake registry.
type Image struct {
	Digest, Version string
}

type deployment struct {
	ref    string
	img    Image
	locked bool // --download-only and not yet --from-downloaded
}

// Fake is safe for concurrent use.
type Fake struct {
	mu sync.Mutex
	// Registry maps a full reference ("repo:tag") to the image it points to.
	Registry map[string]Image
	spec     string
	booted   *deployment
	staged   *deployment
	rollback *deployment
	queued   bool
	cached   *Image
	// OldBootc: do not report downloadOnly in status (bootc before the field).
	OldBootc bool
	// Fail makes a command fail: key is the args joined by spaces
	// ("upgrade --check"), value the stderr text.
	Fail map[string]string
	// Calls records every invocation.
	Calls []string
	// Block, if set, is waited on by mutating commands (tests of "busy").
	Block chan struct{}
}

// New returns a fake booted into ref with image img.
func New(ref string, img Image) *Fake {
	return &Fake{
		Registry: map[string]Image{ref: img},
		spec:     ref,
		booted:   &deployment{ref: ref, img: img},
		Fail:     map[string]string{},
	}
}

// Publish points ref at a new image (a new release on the channel).
func (f *Fake) Publish(ref string, img Image) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.Registry[ref] = img
}

// SetFail makes a command fail ("" clears it).
func (f *Fake) SetFail(cmd, stderr string) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if stderr == "" {
		delete(f.Fail, cmd)
	} else {
		f.Fail[cmd] = stderr
	}
}

// Boot simulates a reboot: a queued rollback wins, then an unlocked staged
// deployment; a locked (download-only) one stays staged.
func (f *Fake) Boot() {
	f.mu.Lock()
	defer f.mu.Unlock()
	switch {
	case f.queued:
		f.booted, f.rollback = f.rollback, f.booted
		f.queued = false
		f.spec = f.booted.ref
	case f.staged != nil && !f.staged.locked:
		f.rollback, f.booted, f.staged = f.booted, f.staged, nil
		f.spec = f.booted.ref
	}
	f.cached = nil
}

// GreenbootRollback simulates a failed boot of the new deployment that
// greenboot rolled back: the staged deployment is lost, the old one runs.
func (f *Fake) GreenbootRollback() {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.staged = nil
	f.cached = nil
}

// Checksum is the ostree checksum the fake reports for a deployment of img.
func Checksum(img Image) string { return "cs-" + strings.TrimPrefix(img.Digest, "sha256:") }

// Booted returns the booted digest.
func (f *Fake) Booted() string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.booted.img.Digest
}

// CallList returns a copy of the calls.
func (f *Fake) CallList() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]string(nil), f.Calls...)
}

// Run implements bootc.Runner.
func (f *Fake) Run(ctx context.Context, args ...string) ([]byte, error) {
	cmd := strings.Join(args, " ")
	f.mu.Lock()
	f.Calls = append(f.Calls, cmd)
	block := f.Block
	f.mu.Unlock()
	if block != nil && cmd != "status --format=json" {
		select {
		case <-block:
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if msg, ok := f.Fail[cmd]; ok {
		return nil, &bootc.ExecError{Args: args, ExitCode: 1, Stderr: msg, Err: errors.New("exit status 1")}
	}
	switch {
	case cmd == "status --format=json":
		return f.status()
	case cmd == "upgrade --check":
		img := f.Registry[f.spec]
		if img.Digest != f.booted.img.Digest {
			f.cached = &img
		} else {
			f.cached = nil
		}
		return nil, nil
	case cmd == "upgrade --download-only":
		img := f.Registry[f.spec]
		if img.Digest == f.booted.img.Digest {
			return []byte("No changes in " + f.spec), nil
		}
		f.staged = &deployment{ref: f.spec, img: img, locked: true}
		return nil, nil
	case cmd == "upgrade --from-downloaded":
		if f.staged == nil || !f.staged.locked {
			return nil, &bootc.ExecError{Args: args, ExitCode: 1, Stderr: "no downloaded deployment", Err: errors.New("exit status 1")}
		}
		f.staged.locked = false
		return nil, nil
	case cmd == "upgrade":
		img := f.Registry[f.spec]
		if img.Digest != f.booted.img.Digest {
			f.staged = &deployment{ref: f.spec, img: img}
		}
		return nil, nil
	case len(args) == 2 && args[0] == "switch":
		img, ok := f.Registry[args[1]]
		if !ok {
			return nil, &bootc.ExecError{Args: args, ExitCode: 1, Stderr: "manifest unknown", Err: errors.New("exit status 1")}
		}
		f.spec = args[1]
		f.staged = &deployment{ref: args[1], img: img}
		return nil, nil
	case cmd == "rollback":
		if f.rollback == nil {
			return nil, &bootc.ExecError{Args: args, ExitCode: 1, Stderr: "No rollback available", Err: errors.New("exit status 1")}
		}
		f.queued = !f.queued
		return nil, nil
	}
	return nil, fmt.Errorf("fakebootc: unknown command %q", cmd)
}

func (f *Fake) entry(d *deployment, withCache bool) map[string]any {
	if d == nil {
		return nil
	}
	e := map[string]any{
		"image": map[string]any{
			"image":       map[string]any{"image": d.ref, "transport": "registry"},
			"version":     d.img.Version,
			"timestamp":   "2026-11-20T03:00:00Z",
			"imageDigest": d.img.Digest,
		},
		"cachedUpdate": nil,
		"incompatible": false,
		"pinned":       false,
		"store":        "ostreeContainer",
		"ostree": map[string]any{
			"checksum": Checksum(d.img), "deploySerial": 0, "stateroot": "default",
		},
	}
	if d.locked && !f.OldBootc {
		e["downloadOnly"] = true
	}
	if withCache && f.cached != nil {
		e["cachedUpdate"] = map[string]any{
			"image":       map[string]any{"image": f.spec, "transport": "registry"},
			"version":     f.cached.Version,
			"imageDigest": f.cached.Digest,
		}
	}
	return e
}

func (f *Fake) status() ([]byte, error) {
	doc := map[string]any{
		"apiVersion": "org.containers.bootc/v1",
		"kind":       "BootcHost",
		"metadata":   map[string]any{"name": "host"},
		"spec": map[string]any{
			"image":     map[string]any{"image": f.spec, "transport": "registry"},
			"bootOrder": "default",
		},
		"status": map[string]any{
			"staged":         f.entry(f.staged, false),
			"booted":         f.entry(f.booted, true),
			"rollback":       f.entry(f.rollback, false),
			"rollbackQueued": f.queued,
			"type":           "bootcHost",
		},
	}
	return json.Marshal(doc)
}
