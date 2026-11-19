#include <malloc.h>

#include "my_application.h"

int main(int argc, char** argv) {
  // BUG-001: at most two glibc malloc arenas. Every player starts about a
  // hundred libmpv/FFmpeg threads; with an arena per thread (up to 8 per
  // core) glibc kept the freed buffers of every closed player, and memory
  // grew with each open and close. Set before any thread starts.
  mallopt(M_ARENA_MAX, 2);
  g_autoptr(MyApplication) app = my_application_new();
  return g_application_run(G_APPLICATION(app), argc, argv);
}
