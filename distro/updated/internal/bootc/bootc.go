// SPDX-License-Identifier: MPL-2.0

// Package bootc drives the bootc CLI: status, check, download-only, apply,
// switch and rollback. Everything goes through a Runner, so tests use a fake
// bootc instead of the real binary.
package bootc

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os/exec"
	"strings"
	"syscall"
	"time"
)

// Runner runs the bootc binary with the given arguments and returns stdout.
type Runner interface {
	Run(ctx context.Context, args ...string) ([]byte, error)
}

// ExecRunner runs a real binary (normally /usr/bin/bootc).
type ExecRunner struct {
	Path string
}

// ExecError is a failed bootc invocation with the tail of its stderr.
type ExecError struct {
	Args     []string
	ExitCode int
	Stderr   string
	Err      error
}

func (e *ExecError) Error() string {
	msg := strings.TrimSpace(e.Stderr)
	if msg == "" && e.Err != nil {
		msg = e.Err.Error()
	}
	return fmt.Sprintf("bootc %s: exit %d: %s", strings.Join(e.Args, " "), e.ExitCode, msg)
}

func (e *ExecError) Unwrap() error { return e.Err }

// maxStderr keeps error messages short enough for logs and API answers.
const maxStderr = 2048

// Run implements Runner.
func (r ExecRunner) Run(ctx context.Context, args ...string) ([]byte, error) {
	path := r.Path
	if path == "" {
		path = "bootc"
	}
	cmd := exec.CommandContext(ctx, path, args...)
	// On shutdown ask bootc to stop (SIGTERM) and give it time to end its
	// ostree transaction cleanly instead of killing it outright.
	cmd.Cancel = func() error { return cmd.Process.Signal(syscall.SIGTERM) }
	cmd.WaitDelay = 60 * time.Second
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		code := -1
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			code = exitErr.ExitCode()
		}
		tail := stderr.String()
		if len(tail) > maxStderr {
			tail = tail[len(tail)-maxStderr:]
		}
		return stdout.Bytes(), &ExecError{Args: args, ExitCode: code, Stderr: tail, Err: err}
	}
	return stdout.Bytes(), nil
}

// ImageReference is the image a deployment tracks (bootc `ImageReference`).
type ImageReference struct {
	Image     string `json:"image"`
	Transport string `json:"transport"`
}

// ImageStatus describes a pulled image (bootc `ImageStatus`).
type ImageStatus struct {
	Image        ImageReference `json:"image"`
	Version      string         `json:"version,omitempty"`
	Timestamp    string         `json:"timestamp,omitempty"`
	ImageDigest  string         `json:"imageDigest"`
	Architecture string         `json:"architecture,omitempty"`
}

// BootEntry is one deployment (booted, staged or rollback).
type BootEntry struct {
	Image        *ImageStatus `json:"image"`
	CachedUpdate *ImageStatus `json:"cachedUpdate"`
	Incompatible bool         `json:"incompatible"`
	Pinned       bool         `json:"pinned"`
	// DownloadOnly is set on a staged deployment made by
	// `bootc upgrade --download-only`: it is not applied on reboot until
	// `bootc upgrade --from-downloaded` unlocks it. Older bootc omits it.
	DownloadOnly bool `json:"downloadOnly,omitempty"`
	// Ostree identifies the deployment on disk (/ostree/deploy/<stateroot>/deploy/<checksum>.<serial>).
	Ostree *OstreeDeployment `json:"ostree,omitempty"`
}

// OstreeDeployment is the ostree part of a boot entry.
type OstreeDeployment struct {
	Checksum     string `json:"checksum"`
	DeploySerial int    `json:"deploySerial"`
	Stateroot    string `json:"stateroot"`
}

// Checksum returns the ostree deployment checksum of the entry or "".
func (e *BootEntry) Checksum() string {
	if e == nil || e.Ostree == nil {
		return ""
	}
	return e.Ostree.Checksum
}

// Digest returns the image digest of the entry or "".
func (e *BootEntry) Digest() string {
	if e == nil || e.Image == nil {
		return ""
	}
	return e.Image.ImageDigest
}

// Host is the output of `bootc status --format=json` (apiVersion org.containers.bootc/v1).
type Host struct {
	APIVersion string `json:"apiVersion"`
	Kind       string `json:"kind"`
	Spec       struct {
		Image *ImageReference `json:"image"`
	} `json:"spec"`
	Status struct {
		Staged         *BootEntry `json:"staged"`
		Booted         *BootEntry `json:"booted"`
		Rollback       *BootEntry `json:"rollback"`
		RollbackQueued bool       `json:"rollbackQueued"`
		Type           string     `json:"type"`
	} `json:"status"`
}

// ParseStatus decodes `bootc status --format=json`.
func ParseStatus(data []byte) (*Host, error) {
	var h Host
	if err := json.Unmarshal(data, &h); err != nil {
		return nil, fmt.Errorf("parse bootc status: %w", err)
	}
	if h.Kind != "" && h.Kind != "BootcHost" {
		return nil, fmt.Errorf("parse bootc status: unexpected kind %q", h.Kind)
	}
	if h.Status.Booted == nil {
		return nil, errors.New("parse bootc status: no booted deployment (not a bootc system?)")
	}
	return &h, nil
}

// Available returns the update found by the last `bootc upgrade --check`
// that is neither booted nor already staged, or nil.
func (h *Host) Available() *ImageStatus {
	b := h.Status.Booted
	if b == nil || b.CachedUpdate == nil || b.CachedUpdate.ImageDigest == "" {
		return nil
	}
	if b.CachedUpdate.ImageDigest == b.Digest() {
		return nil
	}
	if h.Status.Staged.Digest() == b.CachedUpdate.ImageDigest {
		return nil
	}
	return b.CachedUpdate
}

// Client wraps the bootc operations used by alpha-updated.
type Client struct {
	R Runner
	// EnforceSignaturePolicy adds --enforce-container-sigpolicy to `switch`:
	// without it bootc writes the unverified transport into the origin and
	// a channel change would turn signature checks off. Set it only when the
	// booted image ships our policy (ALPHA_SIGNATURE_POLICY=enforce): under
	// the stock policy bootc refuses the signed transport (BUG-031).
	EnforceSignaturePolicy bool
}

// Status runs `bootc status --format=json`.
func (c Client) Status(ctx context.Context) (*Host, error) {
	out, err := c.R.Run(ctx, "status", "--format=json")
	if err != nil {
		return nil, err
	}
	return ParseStatus(out)
}

// Check runs `bootc upgrade --check`: fetches only the manifest and records
// it as the cached update of the booted deployment.
func (c Client) Check(ctx context.Context) error {
	_, err := c.R.Run(ctx, "upgrade", "--check")
	return err
}

// Download runs `bootc upgrade --download-only`: pulls the update and stages
// it locked, so a reboot does not apply it.
func (c Client) Download(ctx context.Context) error {
	_, err := c.R.Run(ctx, "upgrade", "--download-only")
	return err
}

// Unlock runs `bootc upgrade --from-downloaded`: the downloaded deployment is
// applied on the next reboot. No reboot here; the agent reboots itself.
func (c Client) Unlock(ctx context.Context) error {
	_, err := c.R.Run(ctx, "upgrade", "--from-downloaded")
	return err
}

// Upgrade runs `bootc upgrade` (pull and stage, applied on the next reboot).
func (c Client) Upgrade(ctx context.Context) error {
	_, err := c.R.Run(ctx, "upgrade")
	return err
}

// Switch runs `bootc switch IMAGE` (channel change: pull and stage).
func (c Client) Switch(ctx context.Context, image string) error {
	if image == "" || strings.HasPrefix(image, "-") {
		return fmt.Errorf("bootc switch: bad image reference %q", image)
	}
	args := []string{"switch"}
	if c.EnforceSignaturePolicy {
		args = append(args, "--enforce-container-sigpolicy")
	}
	_, err := c.R.Run(ctx, append(args, image)...)
	return err
}

// Rollback runs `bootc rollback`: the previous deployment boots next time.
func (c Client) Rollback(ctx context.Context) error {
	_, err := c.R.Run(ctx, "rollback")
	return err
}

// Repository strips the tag and digest from an image reference:
// "ghcr.io/o/alpha-os:stable" -> "ghcr.io/o/alpha-os",
// "localhost:5000/alpha-os@sha256:…" -> "localhost:5000/alpha-os".
func Repository(ref string) string {
	if i := strings.Index(ref, "@"); i >= 0 {
		ref = ref[:i]
	}
	slash := strings.LastIndex(ref, "/")
	if colon := strings.LastIndex(ref, ":"); colon > slash {
		ref = ref[:colon]
	}
	return ref
}

// Tag returns the tag of an image reference, "" for a digest or no tag.
func Tag(ref string) string {
	if strings.Contains(ref, "@") {
		return ""
	}
	slash := strings.LastIndex(ref, "/")
	if colon := strings.LastIndex(ref, ":"); colon > slash {
		return ref[colon+1:]
	}
	return ""
}
