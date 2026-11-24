// SPDX-License-Identifier: MPL-2.0
//
// TWI-029 (ADR-NNNN): the official Twitch embed on Linux in WebKitGTK.
//
// Flutter on Linux has no platform views, so the web view is a GTK widget
// over the Flutter view: the window's child is a GtkOverlay whose main
// child is the FlView, and the web view is an overlay child placed at the
// rectangle Dart reports (`rect`, logical pixels of the Flutter view, which
// are GTK's). Dart hides it while another route (a dialog) is on top.
//
// WebKitGTK (LGPL-2.1/BSD) is the system's: libwebkit2gtk-4.1 (GTK 3, like
// the app), or the older 4.0 ABI, opened with dlopen. Nothing links to it,
// so the app, its AppImage and its .deb start without it; `available`
// answers false and Dart offers "Open on Twitch" with a hint to install it.
// Video decoding is WebKit's GStreamer (H.264 from the system's plugins).
//
// Channel `app.kinolune/twitch_webview` (standard codec):
//   Dart -> native: available -> bool; create {html, baseUrl} -> bool;
//     rect {x, y, width, height, visible}; runJavaScript {script}; dispose.
//   native -> Dart: message (string from the page), external (URL that
//     would leave the page), fullscreen (bool), back (Escape in the view),
//     failed (the web process died).
// The page is loaded from a string with the project's HTTPS base URL, as
// on Android and macOS (ADR-0051), so the embed sees its `parent`.

#include "twitch_webview.h"

#include <dlfcn.h>
#include <gdk/gdkkeysyms.h>

#include <string.h>

namespace {

constexpr char kChannel[] = "app.kinolune/twitch_webview";
constexpr char kHandler[] = "KinoluneEmbed";

// WebKitGTK enums (stable ABI of webkit2gtk-4.0/4.1).
constexpr int kPolicyNewWindowAction = 1;
constexpr int kInjectTopFrame = 1;
constexpr int kInjectAtStart = 0;
constexpr int kLoadCommitted = 2;

typedef struct _WebKitUserContentManager WebKitUserContentManager;
typedef struct _WebKitUserScript WebKitUserScript;
typedef struct _WebKitSettings WebKitSettings;
typedef struct _WebKitJavascriptResult WebKitJavascriptResult;
typedef struct _JSCValue JSCValue;

struct WebKit {
  GtkWidget* (*web_view_new_with_user_content_manager)(
      WebKitUserContentManager*);
  WebKitUserContentManager* (*user_content_manager_new)();
  gboolean (*register_script_message_handler)(WebKitUserContentManager*,
                                              const gchar*);
  void (*add_script)(WebKitUserContentManager*, WebKitUserScript*);
  WebKitUserScript* (*user_script_new)(const gchar*, int, int,
                                       const gchar* const*,
                                       const gchar* const*);
  void (*user_script_unref)(WebKitUserScript*);
  void (*load_html)(GtkWidget*, const gchar*, const gchar*);
  WebKitSettings* (*get_settings)(GtkWidget*);
  void (*set_media_playback_requires_user_gesture)(WebKitSettings*, gboolean);
  void (*set_background_color)(GtkWidget*, const GdkRGBA*);
  const gchar* (*get_uri)(GtkWidget*);
  void (*run_javascript)(GtkWidget*, const gchar*, GCancellable*,
                         GAsyncReadyCallback, gpointer);
  void (*policy_decision_ignore)(gpointer);
  gpointer (*navigation_policy_decision_get_navigation_action)(gpointer);
  gpointer (*navigation_action_get_request)(gpointer);
  const gchar* (*uri_request_get_uri)(gpointer);
  JSCValue* (*javascript_result_get_js_value)(WebKitJavascriptResult*);
  gchar* (*jsc_value_to_string)(JSCValue*);
};

struct State {
  FlMethodChannel* channel = nullptr;
  GtkOverlay* overlay = nullptr;
  FlView* view = nullptr;
  GtkWidget* web_view = nullptr;
  gchar* base_url = nullptr;
  gchar* html = nullptr;
  GdkRectangle rect = {0, 0, 0, 0};
  gboolean visible = FALSE;
  gboolean fullscreen = FALSE;
};

State state;
WebKit webkit;
// 0 = not tried, 1 = loaded, -1 = missing.
int webkit_status = 0;

template <typename T>
bool resolve(void* lib, const char* name, T* out) {
  *out = reinterpret_cast<T>(dlsym(lib, name));
  return *out != nullptr;
}

bool load_webkit() {
  if (webkit_status != 0) {
    return webkit_status > 0;
  }
  webkit_status = -1;
  const gchar* off = g_getenv("KINOLUNE_TWITCH_WEBKIT");
  if (off != nullptr && g_strcmp0(off, "0") == 0) {
    return false;
  }
  // GTK 3 ABIs only: libwebkitgtk-6.0 is GTK 4 and cannot share a process
  // with this GTK 3 window.
  void* lib = nullptr;
  for (const char* name :
       {"libwebkit2gtk-4.1.so.0", "libwebkit2gtk-4.0.so.37"}) {
    lib = dlopen(name, RTLD_NOW | RTLD_LOCAL);
    if (lib != nullptr) {
      break;
    }
  }
  if (lib == nullptr) {
    g_message("Twitch embed: WebKitGTK not found (%s)", dlerror());
    return false;
  }
  WebKit w = {};
  bool ok =
      resolve(lib, "webkit_web_view_new_with_user_content_manager",
              &w.web_view_new_with_user_content_manager) &&
      resolve(lib, "webkit_user_content_manager_new",
              &w.user_content_manager_new) &&
      resolve(lib, "webkit_user_content_manager_register_script_message_handler",
              &w.register_script_message_handler) &&
      resolve(lib, "webkit_user_content_manager_add_script", &w.add_script) &&
      resolve(lib, "webkit_user_script_new", &w.user_script_new) &&
      resolve(lib, "webkit_user_script_unref", &w.user_script_unref) &&
      resolve(lib, "webkit_web_view_load_html", &w.load_html) &&
      resolve(lib, "webkit_web_view_get_settings", &w.get_settings) &&
      resolve(lib, "webkit_settings_set_media_playback_requires_user_gesture",
              &w.set_media_playback_requires_user_gesture) &&
      resolve(lib, "webkit_web_view_set_background_color",
              &w.set_background_color) &&
      resolve(lib, "webkit_web_view_get_uri", &w.get_uri) &&
      resolve(lib, "webkit_web_view_run_javascript", &w.run_javascript) &&
      resolve(lib, "webkit_policy_decision_ignore",
              &w.policy_decision_ignore) &&
      resolve(lib, "webkit_navigation_policy_decision_get_navigation_action",
              &w.navigation_policy_decision_get_navigation_action) &&
      resolve(lib, "webkit_navigation_action_get_request",
              &w.navigation_action_get_request) &&
      resolve(lib, "webkit_uri_request_get_uri", &w.uri_request_get_uri) &&
      resolve(lib, "webkit_javascript_result_get_js_value",
              &w.javascript_result_get_js_value) &&
      resolve(lib, "jsc_value_to_string", &w.jsc_value_to_string);
  if (!ok) {
    g_message("Twitch embed: WebKitGTK lacks %s", dlerror());
    return false;
  }
  webkit = w;
  webkit_status = 1;
  return true;
}

void invoke(const gchar* method, FlValue* args) {
  if (state.channel == nullptr) {
    return;
  }
  fl_method_channel_invoke_method(state.channel, method, args, nullptr,
                                  nullptr, nullptr);
}

void invoke_string(const gchar* method, const gchar* value) {
  g_autoptr(FlValue) v = fl_value_new_string(value != nullptr ? value : "");
  invoke(method, v);
}

void destroy_web_view() {
  if (state.web_view == nullptr) {
    return;
  }
  GtkWidget* view = state.web_view;
  state.web_view = nullptr;
  g_signal_handlers_disconnect_matched(view, G_SIGNAL_MATCH_DATA, 0, 0,
                                       nullptr, nullptr, &state);
  gtk_widget_destroy(view);
  if (state.fullscreen) {
    state.fullscreen = FALSE;
    GtkWidget* top = gtk_widget_get_toplevel(GTK_WIDGET(state.overlay));
    if (GTK_IS_WINDOW(top)) {
      gtk_window_unfullscreen(GTK_WINDOW(top));
    }
  }
  g_clear_pointer(&state.base_url, g_free);
  g_clear_pointer(&state.html, g_free);
  state.visible = FALSE;
  gtk_widget_grab_focus(GTK_WIDGET(state.view));
}

void apply_rect() {
  if (state.web_view == nullptr) {
    return;
  }
  const gboolean show =
      state.visible && state.rect.width > 0 && state.rect.height > 0;
  gtk_widget_set_visible(state.web_view, show);
  gtk_widget_queue_resize(GTK_WIDGET(state.overlay));
}

gboolean get_child_position_cb(GtkOverlay* overlay, GtkWidget* widget,
                               GdkRectangle* allocation, gpointer data) {
  if (widget != state.web_view) {
    return FALSE;
  }
  *allocation = state.rect;
  return TRUE;
}

void script_message_cb(WebKitUserContentManager* manager,
                       WebKitJavascriptResult* result, gpointer data) {
  JSCValue* value = webkit.javascript_result_get_js_value(result);
  g_autofree gchar* text =
      value != nullptr ? webkit.jsc_value_to_string(value) : nullptr;
  if (text != nullptr) {
    invoke_string("message", text);
  }
}

// Links that open a new window (the Twitch logo, a channel name) go to the
// browser; the page itself never leaves its base URL.
gboolean decide_policy_cb(GtkWidget* view, gpointer decision, int type,
                          gpointer data) {
  if (type != kPolicyNewWindowAction) {
    return FALSE;
  }
  gpointer action =
      webkit.navigation_policy_decision_get_navigation_action(decision);
  gpointer request =
      action != nullptr ? webkit.navigation_action_get_request(action)
                        : nullptr;
  const gchar* uri =
      request != nullptr ? webkit.uri_request_get_uri(request) : nullptr;
  if (uri != nullptr && g_str_has_prefix(uri, "https://")) {
    invoke_string("external", uri);
  }
  webkit.policy_decision_ignore(decision);
  return TRUE;
}

void load_changed_cb(GtkWidget* view, int event, gpointer data) {
  if (event != kLoadCommitted || state.base_url == nullptr) {
    return;
  }
  const gchar* uri = webkit.get_uri(view);
  if (uri == nullptr || g_strcmp0(uri, state.base_url) == 0 ||
      g_str_has_prefix(uri, "about:")) {
    return;
  }
  // The main frame navigated away (a link with target=_top): back to the
  // player, the link to the browser.
  if (g_str_has_prefix(uri, "https://")) {
    invoke_string("external", uri);
  }
  webkit.load_html(view, state.html, state.base_url);
}

gboolean enter_fullscreen_cb(GtkWidget* view, gpointer data) {
  state.fullscreen = TRUE;
  g_autoptr(FlValue) v = fl_value_new_bool(TRUE);
  invoke("fullscreen", v);
  return FALSE;  // WebKit fullscreens the window itself.
}

gboolean leave_fullscreen_cb(GtkWidget* view, gpointer data) {
  state.fullscreen = FALSE;
  g_autoptr(FlValue) v = fl_value_new_bool(FALSE);
  invoke("fullscreen", v);
  return FALSE;
}

gboolean key_press_cb(GtkWidget* view, GdkEventKey* event, gpointer data) {
  if (event->keyval == GDK_KEY_Escape && !state.fullscreen) {
    invoke("back", nullptr);
    return TRUE;
  }
  return FALSE;
}

void web_process_terminated_cb(GtkWidget* view, int reason, gpointer data) {
  invoke("failed", nullptr);
}

bool create_web_view(const gchar* html, const gchar* base_url) {
  destroy_web_view();
  if (!load_webkit()) {
    return false;
  }
  WebKitUserContentManager* manager = webkit.user_content_manager_new();
  webkit.register_script_message_handler(manager, kHandler);
  // The page posts to `KinoluneEmbed.postMessage`, as webview_flutter
  // names its channel on Android, iOS and macOS.
  WebKitUserScript* script = webkit.user_script_new(
      "window.KinoluneEmbed = {postMessage: function (m) {"
      " window.webkit.messageHandlers.KinoluneEmbed.postMessage(m); }};",
      kInjectTopFrame, kInjectAtStart, nullptr, nullptr);
  webkit.add_script(manager, script);
  webkit.user_script_unref(script);
  g_autofree gchar* signal =
      g_strdup_printf("script-message-received::%s", kHandler);
  g_signal_connect(manager, signal, G_CALLBACK(script_message_cb), &state);

  GtkWidget* view = webkit.web_view_new_with_user_content_manager(manager);
  g_object_unref(manager);
  webkit.set_media_playback_requires_user_gesture(webkit.get_settings(view),
                                                  FALSE);
  GdkRGBA black = {0, 0, 0, 1};
  webkit.set_background_color(view, &black);
  g_signal_connect(view, "decide-policy", G_CALLBACK(decide_policy_cb),
                   &state);
  g_signal_connect(view, "load-changed", G_CALLBACK(load_changed_cb), &state);
  g_signal_connect(view, "enter-fullscreen", G_CALLBACK(enter_fullscreen_cb),
                   &state);
  g_signal_connect(view, "leave-fullscreen", G_CALLBACK(leave_fullscreen_cb),
                   &state);
  g_signal_connect(view, "key-press-event", G_CALLBACK(key_press_cb), &state);
  g_signal_connect(view, "web-process-terminated",
                   G_CALLBACK(web_process_terminated_cb), &state);

  state.web_view = view;
  state.base_url = g_strdup(base_url);
  state.html = g_strdup(html);
  gtk_overlay_add_overlay(state.overlay, view);
  apply_rect();
  webkit.load_html(view, html, base_url);
  return true;
}

const gchar* string_arg(FlValue* args, const char* key) {
  if (args == nullptr || fl_value_get_type(args) != FL_VALUE_TYPE_MAP) {
    return nullptr;
  }
  FlValue* v = fl_value_lookup_string(args, key);
  return v != nullptr && fl_value_get_type(v) == FL_VALUE_TYPE_STRING
             ? fl_value_get_string(v)
             : nullptr;
}

double number_arg(FlValue* args, const char* key) {
  FlValue* v = args != nullptr && fl_value_get_type(args) == FL_VALUE_TYPE_MAP
                   ? fl_value_lookup_string(args, key)
                   : nullptr;
  if (v == nullptr) {
    return 0;
  }
  switch (fl_value_get_type(v)) {
    case FL_VALUE_TYPE_FLOAT:
      return fl_value_get_float(v);
    case FL_VALUE_TYPE_INT:
      return static_cast<double>(fl_value_get_int(v));
    default:
      return 0;
  }
}

void method_cb(FlMethodChannel* channel, FlMethodCall* call, gpointer data) {
  const gchar* name = fl_method_call_get_name(call);
  FlValue* args = fl_method_call_get_args(call);
  g_autoptr(FlValue) result = nullptr;
  if (g_strcmp0(name, "available") == 0) {
    result = fl_value_new_bool(load_webkit());
  } else if (g_strcmp0(name, "create") == 0) {
    const gchar* html = string_arg(args, "html");
    const gchar* base_url = string_arg(args, "baseUrl");
    result = fl_value_new_bool(html != nullptr && base_url != nullptr &&
                               create_web_view(html, base_url));
  } else if (g_strcmp0(name, "rect") == 0) {
    state.rect.x = static_cast<int>(number_arg(args, "x") + 0.5);
    state.rect.y = static_cast<int>(number_arg(args, "y") + 0.5);
    state.rect.width = static_cast<int>(number_arg(args, "width") + 0.5);
    state.rect.height = static_cast<int>(number_arg(args, "height") + 0.5);
    FlValue* visible = fl_value_lookup_string(args, "visible");
    state.visible = visible != nullptr &&
                    fl_value_get_type(visible) == FL_VALUE_TYPE_BOOL &&
                    fl_value_get_bool(visible);
    apply_rect();
  } else if (g_strcmp0(name, "runJavaScript") == 0) {
    const gchar* script = string_arg(args, "script");
    if (state.web_view != nullptr && script != nullptr) {
      webkit.run_javascript(state.web_view, script, nullptr, nullptr,
                            nullptr);
    }
  } else if (g_strcmp0(name, "dispose") == 0) {
    destroy_web_view();
  } else {
    fl_method_call_respond_not_implemented(call, nullptr);
    return;
  }
  fl_method_call_respond_success(call, result, nullptr);
}

}  // namespace

void twitch_webview_register(FlView* view, GtkOverlay* overlay) {
  state.view = view;
  state.overlay = overlay;
  g_signal_connect(overlay, "get-child-position",
                   G_CALLBACK(get_child_position_cb), &state);
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  state.channel = fl_method_channel_new(
      fl_engine_get_binary_messenger(fl_view_get_engine(view)), kChannel,
      FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(state.channel, method_cb, nullptr,
                                            nullptr);
}
