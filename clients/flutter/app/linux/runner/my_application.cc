#include "my_application.h"

#include <flutter_linux/flutter_linux.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include <string.h>

#include "flutter/generated_plugin_registrant.h"

// UI-022 (ADR-NNNN): one copy of the app per user. GApplication owns the
// D-Bus name APPLICATION_ID; a second start hands its command line to the
// running copy (`command-line` below) and quits. The running copy brings its
// window to the front and gives the arguments (`kinolune://` links, files
// from "Open with") to Dart on the channel `app.kinolune/links`
// (lib/platform/app_links.dart). Without a session bus every start runs on
// its own, as before.

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
  GtkWindow* window;
  FlMethodChannel* links_channel;
  // Arguments of second starts Dart has not taken yet.
  GPtrArray* pending_links;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// Called when first Flutter frame received.
static void first_frame_cb(MyApplication* self, FlView* view) {
  gtk_widget_show(gtk_widget_get_toplevel(GTK_WIDGET(view)));
}

// Default window size. Under a window manager the window opens at 1280x800
// and the user resizes, maximizes or fullscreens it. Without one (a bare X
// session on an HTPC, a kiosk, Xvfb) nothing can resize it, and
// gtk_window_fullscreen() (the player's fullscreen button) is a no-op, so the
// app and its player would stay in a 1280x800 corner of a larger screen: the
// window fills the monitor's work area instead.
static void set_initial_size(GtkWindow* window) {
  gint width = 1280;
  gint height = 800;
#ifdef GDK_WINDOWING_X11
  GdkDisplay* display = gdk_display_get_default();
  GdkScreen* screen = gdk_screen_get_default();
  if (GDK_IS_X11_DISPLAY(display) && screen != nullptr &&
      !gdk_x11_screen_supports_net_wm_hint(
          screen, gdk_atom_intern_static_string("_NET_WM_STATE_FULLSCREEN"))) {
    GdkMonitor* monitor = gdk_display_get_primary_monitor(display);
    if (monitor == nullptr && gdk_display_get_n_monitors(display) > 0) {
      monitor = gdk_display_get_monitor(display, 0);
    }
    if (monitor != nullptr) {
      GdkRectangle area;
      gdk_monitor_get_workarea(monitor, &area);
      if (area.width > 0 && area.height > 0) {
        width = area.width;
        height = area.height;
        gtk_window_move(window, area.x, area.y);
      }
    }
  }
#endif
  gtk_window_set_default_size(window, width, height);
}

// UI-022: native window decorations. GNOME (and the desktops built like it)
// draws the title bar inside the window, so there the window gets a GTK
// header bar; KDE Plasma, Xfce, Cinnamon, MATE and the rest decorate windows
// themselves (server side), so the window keeps a plain title. Without a
// desktop (a kiosk) there is nothing to match. KINOLUNE_HEADER_BAR=1 or 0
// forces either.
static gboolean use_header_bar() {
  const gchar* forced = g_getenv("KINOLUNE_HEADER_BAR");
  if (forced != nullptr && forced[0] != '\0') {
    return g_strcmp0(forced, "1") == 0;
  }
  const gchar* desktop = g_getenv("XDG_CURRENT_DESKTOP");
  if (desktop == nullptr) {
    return FALSE;
  }
  g_auto(GStrv) names = g_strsplit(desktop, ":", -1);
  for (gchar** name = names; *name != nullptr; name++) {
    if (g_ascii_strcasecmp(*name, "GNOME") == 0 ||
        g_ascii_strcasecmp(*name, "Unity") == 0 ||
        g_ascii_strcasecmp(*name, "Pantheon") == 0 ||
        g_ascii_strcasecmp(*name, "Budgie") == 0) {
      return TRUE;
    }
  }
  return FALSE;
}

// `take` (Dart): the arguments that came before Dart listened.
static void links_method_cb(FlMethodChannel* channel, FlMethodCall* call,
                            gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  if (g_strcmp0(fl_method_call_get_name(call), "take") != 0) {
    fl_method_call_respond_not_implemented(call, nullptr);
    return;
  }
  g_autoptr(FlValue) list = fl_value_new_list();
  for (guint i = 0; i < self->pending_links->len; i++) {
    fl_value_append_take(
        list, fl_value_new_string(static_cast<const gchar*>(
                  g_ptr_array_index(self->pending_links, i))));
  }
  g_ptr_array_set_size(self->pending_links, 0);
  fl_method_call_respond_success(call, list, nullptr);
}

typedef struct {
  MyApplication* self;
  GStrv args;
} OpenLinks;

static void keep_pending(MyApplication* self, GStrv args) {
  for (gchar** a = args; *a != nullptr; a++) {
    g_ptr_array_add(self->pending_links, g_strdup(*a));
  }
}

// Dart answers `open` with true once it took the arguments; otherwise they
// wait for `take`.
static void links_open_cb(GObject* object, GAsyncResult* result,
                          gpointer user_data) {
  OpenLinks* open = static_cast<OpenLinks*>(user_data);
  g_autoptr(GError) error = nullptr;
  g_autoptr(FlMethodResponse) response = fl_method_channel_invoke_method_finish(
      FL_METHOD_CHANNEL(object), result, &error);
  FlValue* value = response == nullptr
                       ? nullptr
                       : fl_method_response_get_result(response, nullptr);
  const gboolean taken = value != nullptr &&
                         fl_value_get_type(value) == FL_VALUE_TYPE_BOOL &&
                         fl_value_get_bool(value);
  if (!taken) {
    keep_pending(open->self, open->args);
  }
  g_strfreev(open->args);
  g_object_unref(open->self);
  g_free(open);
}

static void open_links(MyApplication* self, GStrv args) {
  if (args == nullptr || args[0] == nullptr) {
    return;
  }
  if (self->links_channel == nullptr) {
    keep_pending(self, args);
    return;
  }
  g_autoptr(FlValue) list = fl_value_new_list();
  for (gchar** a = args; *a != nullptr; a++) {
    fl_value_append_take(list, fl_value_new_string(*a));
  }
  OpenLinks* open = g_new0(OpenLinks, 1);
  open->self = MY_APPLICATION(g_object_ref(self));
  open->args = g_strdupv(args);
  fl_method_channel_invoke_method(self->links_channel, "open", list, nullptr,
                                  links_open_cb, open);
}

// Implements GApplication::activate: the window, once; later activations
// (a second start, a desktop launcher) bring it to the front.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  if (self->window != nullptr) {
    gtk_window_present(self->window);
    return;
  }

  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));
  self->window = window;
  g_object_add_weak_pointer(G_OBJECT(window),
                            reinterpret_cast<gpointer*>(&self->window));
  set_initial_size(window);

  // The app is always dark: so are its GTK parts (the header bar, file
  // choosers).
  g_object_set(gtk_settings_get_default(), "gtk-application-prefer-dark-theme",
               TRUE, nullptr);
  if (use_header_bar()) {
    GtkHeaderBar* header_bar = GTK_HEADER_BAR(gtk_header_bar_new());
    gtk_widget_show(GTK_WIDGET(header_bar));
    gtk_header_bar_set_title(header_bar, "Kinolune");
    gtk_header_bar_set_show_close_button(header_bar, TRUE);
    gtk_window_set_titlebar(window, GTK_WIDGET(header_bar));
  }
  gtk_window_set_title(window, "Kinolune");
  gtk_window_set_icon_name(window, "app.kinolune");

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);
  // Impeller (OpenGL ES) draws the UI into a quarter of the window on HiDPI
  // Linux desktops (0.1.0-alpha.5 on Ubuntu, both Wayland and X11), so the
  // Linux app renders with Skia. KINOLUNE_IMPELLER=1 opts back into Impeller,
  // e.g. to check a newer Flutter engine.
  const gchar* impeller = g_getenv("KINOLUNE_IMPELLER");
  fl_dart_project_set_enable_impeller(project,
                                      g_strcmp0(impeller, "1") == 0);

  FlView* view = fl_view_new(project);
  GdkRGBA background_color;
  // Background defaults to black, override it here if necessary, e.g. #00000000
  // for transparent.
  gdk_rgba_parse(&background_color, "#0B0B0F");
  fl_view_set_background_color(view, &background_color);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  // Show the window when Flutter renders.
  // Requires the view to be realized so we can start rendering.
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));

  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  self->links_channel = fl_method_channel_new(
      fl_engine_get_binary_messenger(fl_view_get_engine(view)),
      "app.kinolune/links", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(self->links_channel,
                                            links_method_cb, self, nullptr);

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

// The arguments of a start without the program name; relative file paths
// become absolute (a second start runs in another directory).
static GStrv command_line_arguments(GApplicationCommandLine* command_line) {
  gint argc = 0;
  g_auto(GStrv) argv =
      g_application_command_line_get_arguments(command_line, &argc);
  GPtrArray* out = g_ptr_array_new();
  for (gint i = 1; i < argc; i++) {
    const gchar* arg = argv[i];
    gchar* value = nullptr;
    if (arg[0] != '-' && strstr(arg, "://") == nullptr &&
        !g_str_has_prefix(arg, "file:")) {
      g_autoptr(GFile) file =
          g_application_command_line_create_file_for_arg(command_line, arg);
      value = g_file_get_path(file);
    }
    g_ptr_array_add(out, value != nullptr ? value : g_strdup(arg));
  }
  g_ptr_array_add(out, nullptr);
  return reinterpret_cast<GStrv>(g_ptr_array_free(out, FALSE));
}

// Implements GApplication::command_line: runs in the first copy for its own
// start and for every later one.
static gint my_application_command_line(GApplication* application,
                                        GApplicationCommandLine* command_line) {
  MyApplication* self = MY_APPLICATION(application);
  g_auto(GStrv) args = command_line_arguments(command_line);
  if (self->window == nullptr) {
    g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
    self->dart_entrypoint_arguments = g_strdupv(args);
    g_application_activate(application);
  } else {
    gtk_window_present(self->window);
    open_links(self, args);
  }
  return 0;
}

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application startup.

  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application shutdown.

  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

// Implements GObject::dispose.
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  g_clear_object(&self->links_channel);
  g_clear_pointer(&self->pending_links, g_ptr_array_unref);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->command_line = my_application_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {
  self->pending_links = g_ptr_array_new_with_free_func(g_free);
}

MyApplication* my_application_new() {
  // Set the program name to the application ID, which helps various systems
  // like GTK and desktop environments map this running application to its
  // corresponding .desktop file. This ensures better integration by allowing
  // the application to be recognized beyond its binary name.
  g_set_prgname(APPLICATION_ID);

  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID, "flags",
                                     G_APPLICATION_HANDLES_COMMAND_LINE,
                                     nullptr));
}
