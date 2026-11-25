// SPDX-License-Identifier: MPL-2.0

package install

import (
	"bytes"
	"context"
	"fmt"
	"io"
	"os/exec"
	"strings"
)

// Cmd is one external command of the installation.
type Cmd struct {
	Name  string
	Args  []string
	Stdin string
	// SecretStdin keeps Stdin out of logs and plans (passwords).
	SecretStdin bool
}

func (c Cmd) String() string {
	parts := append([]string{c.Name}, c.Args...)
	for i, p := range parts {
		if p == "" || strings.ContainsAny(p, " \t\"'$") {
			parts[i] = fmt.Sprintf("%q", p)
		}
	}
	s := strings.Join(parts, " ")
	switch {
	case c.SecretStdin:
		s += " <<< (secret)"
	case c.Stdin != "":
		s += " <<EOF\n" + c.Stdin + "EOF"
	}
	return s
}

// Runner runs external commands. Tests replace it with a fake that records
// commands and emulates their effects on a temporary directory.
type Runner interface {
	Run(ctx context.Context, c Cmd) (stdout []byte, err error)
	// LookPath reports whether a program is installed.
	LookPath(name string) bool
}

// ExecRunner runs commands for real and echoes them to Log.
type ExecRunner struct {
	Log io.Writer
}

// Run implements Runner.
func (r ExecRunner) Run(ctx context.Context, c Cmd) ([]byte, error) {
	if r.Log != nil {
		line := c.String()
		if i := strings.Index(line, " <<EOF"); i >= 0 {
			line = line[:i] + " <<EOF …"
		}
		fmt.Fprintln(r.Log, "+", line)
	}
	cmd := exec.CommandContext(ctx, c.Name, c.Args...)
	if c.Stdin != "" {
		cmd.Stdin = strings.NewReader(c.Stdin)
	}
	var out, errb bytes.Buffer
	cmd.Stdout = &out
	cmd.Stderr = &errb
	if r.Log != nil {
		// Long steps (bootc) print progress; show it live.
		cmd.Stderr = io.MultiWriter(&errb, r.Log)
	}
	if err := cmd.Run(); err != nil {
		msg := strings.TrimSpace(errb.String())
		if len(msg) > 2000 {
			msg = "…" + msg[len(msg)-2000:]
		}
		return out.Bytes(), fmt.Errorf("%s: %w: %s", c.Name, err, msg)
	}
	return out.Bytes(), nil
}

// LookPath implements Runner.
func (ExecRunner) LookPath(name string) bool {
	_, err := exec.LookPath(name)
	return err == nil
}
