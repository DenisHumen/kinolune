#include <gtk/gtk.h>
#include <stdlib.h>
#include <string.h>

#include "my_application.h"

int main(int argc, char** argv) {
  // Flutter's Impeller (OpenGL ES) renders into a quarter of the window on
  // native Wayland GTK surfaces, so prefer X11 (XWayland on Wayland desktops)
  // and fall back to any backend. KINOLUNE_WAYLAND=1 opts back into native
  // Wayland, e.g. to check a newer Flutter engine.
  const char* wayland = getenv("KINOLUNE_WAYLAND");
  if (wayland == nullptr || strcmp(wayland, "1") != 0) {
    gdk_set_allowed_backends("x11,*");
  }
  g_autoptr(MyApplication) app = my_application_new();
  return g_application_run(G_APPLICATION(app), argc, argv);
}
