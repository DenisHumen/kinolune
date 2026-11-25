// SPDX-License-Identifier: MPL-2.0
//
// TWI-029 (ADR-0144): the official Twitch embed on Linux. A WebKitGTK view
// lies over the Flutter view (GtkOverlay) at the rectangle Dart reports;
// WebKitGTK is loaded at run time (dlopen), so the app starts and works
// without it and Dart falls back to "Open on Twitch".

#ifndef RUNNER_TWITCH_WEBVIEW_H_
#define RUNNER_TWITCH_WEBVIEW_H_

#include <flutter_linux/flutter_linux.h>
#include <gtk/gtk.h>

// Registers the channel `app.kinolune/twitch_webview` of [view]; the web
// view is added to [overlay], whose main child is [view].
void twitch_webview_register(FlView* view, GtkOverlay* overlay);

// BUG-028: whether the web view goes into its own Wayland subsurface (a
// transient popup) instead of the overlay. Flutter draws into a subsurface
// on Wayland, which covers the overlay; on X11 the overlay works. The
// environment variable KINOLUNE_TWITCH_WEBKIT_LAYER (overlay | subsurface)
// overrides the choice.
bool twitch_webview_use_subsurface(GdkDisplay* display);

#endif  // RUNNER_TWITCH_WEBVIEW_H_
