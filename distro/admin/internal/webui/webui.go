// SPDX-License-Identifier: MPL-2.0

// Package webui is the web interface of alpha-admin (DIST-014): static HTML,
// CSS and ES modules embedded in the binary, no build step and no CDN. The
// pages talk only to the documented REST API (internal/server/openapi.yaml).
// Colours come from the generated design tokens (static/tokens.css, a copy of
// design/tokens/generated/web/tokens.css); see docs/adr/NNNN-admin-web-ui.md.
package webui

import (
	"bytes"
	"crypto/sha256"
	"embed"
	"encoding/hex"
	"io/fs"
	"mime"
	"net/http"
	"path"
	"strings"
	"time"
)

//go:embed static
var files embed.FS

// Prefix is the URL path the static files are served under.
const Prefix = "/static/"

type asset struct {
	body  []byte
	ctype string
	etag  string
}

var assets = func() map[string]asset {
	m := map[string]asset{}
	err := fs.WalkDir(files, "static", func(p string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		b, err := files.ReadFile(p)
		if err != nil {
			return err
		}
		sum := sha256.Sum256(b)
		ct := mime.TypeByExtension(path.Ext(p))
		switch path.Ext(p) {
		case ".js":
			ct = "text/javascript; charset=utf-8"
		case ".json":
			ct = "application/json"
		case ".svg":
			ct = "image/svg+xml"
		}
		if ct == "" {
			ct = "application/octet-stream"
		}
		m[strings.TrimPrefix(p, "static/")] = asset{body: b, ctype: ct, etag: `"` + hex.EncodeToString(sum[:8]) + `"`}
		return nil
	})
	if err != nil {
		panic(err)
	}
	return m
}()

// Names lists the embedded files (relative to Prefix), for tests.
func Names() []string {
	out := make([]string, 0, len(assets))
	for k := range assets {
		out = append(out, k)
	}
	return out
}

// File returns an embedded file (relative to Prefix).
func File(name string) ([]byte, bool) {
	a, ok := assets[name]
	return a.body, ok
}

func serve(w http.ResponseWriter, r *http.Request, a asset) {
	h := w.Header()
	h.Set("Content-Type", a.ctype)
	// Revalidate every time: the files change with OS updates and are tiny.
	h.Set("Cache-Control", "no-cache")
	h.Set("ETag", a.etag)
	if match := r.Header.Get("If-None-Match"); match != "" && match == a.etag {
		w.WriteHeader(http.StatusNotModified)
		return
	}
	http.ServeContent(w, r, "", time.Time{}, bytes.NewReader(a.body))
}

// Index serves the single page (index.html) at "/".
func Index(w http.ResponseWriter, r *http.Request) {
	serve(w, r, assets["index.html"])
}

// Static serves GET /static/<file>.
func Static(w http.ResponseWriter, r *http.Request) {
	name := strings.TrimPrefix(r.URL.Path, Prefix)
	a, ok := assets[name]
	if !ok || name == "index.html" {
		http.NotFound(w, r)
		return
	}
	serve(w, r, a)
}
