// SPDX-License-Identifier: MPL-2.0
//
// Audio-device smoke test of a libmpv build (PKG-021). Does what media_kit
// does when a video starts: observes `audio-device-list` (mpv then lists the
// devices of every audio output, CoreAudio included, on its core thread),
// reads it, and plays a file through a real audio output.
//
// alpha.11's macOS libmpv crashed here with SIGSEGV at 0x0 (ca_get_str
// called cfstr_get_cstr, an unresolved dynamic_lookup symbol), so a crash
// of this program is the failure being tested for.
//
//     check_audio_devices <media file> [ao] [seconds]
//
// ao defaults to the platform output (coreaudio on macOS); `null` is the
// fallback when the runner has no audio device. Exit code 0 = OK.
//
// Build (macOS, against the frameworks the app ships):
//     cc check_audio_devices.c -I<libmpv>/include -F<dir> -framework Mpv \
//        -Wl,-rpath,<dir>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <mpv/client.h>

static int failures = 0;

static void expect(int cond, const char *what) {
    printf("%-52s [%s]\n", what, cond ? "ok" : "FAIL");
    fflush(stdout);
    if (!cond)
        failures++;
}

// Prints the devices of an `audio-device-list` node; returns their number.
static int print_devices(const char *when, const mpv_node *list) {
    if (list->format != MPV_FORMAT_NODE_ARRAY) {
        printf("%s: audio-device-list is not an array (format %d)\n", when, list->format);
        return -1;
    }
    int n = list->u.list->num;
    printf("%s: %d audio device(s)\n", when, n);
    for (int i = 0; i < n; i++) {
        const mpv_node *dev = &list->u.list->values[i];
        const char *name = "?", *desc = "?";
        if (dev->format == MPV_FORMAT_NODE_MAP) {
            for (int k = 0; k < dev->u.list->num; k++) {
                const mpv_node *v = &dev->u.list->values[k];
                if (v->format != MPV_FORMAT_STRING)
                    continue;
                if (strcmp(dev->u.list->keys[k], "name") == 0)
                    name = v->u.string;
                else if (strcmp(dev->u.list->keys[k], "description") == 0)
                    desc = v->u.string;
            }
        }
        printf("  %s  (%s)\n", name, desc);
    }
    fflush(stdout);
    return n;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <media file> [ao] [seconds]\n", argv[0]);
        return 2;
    }
    const char *file = argv[1];
#ifdef __APPLE__
    const char *ao = argc > 2 ? argv[2] : "coreaudio";
#else
    const char *ao = argc > 2 ? argv[2] : "";
#endif
    double seconds = argc > 3 ? atof(argv[3]) : 10.0;

    mpv_handle *mpv = mpv_create();
    expect(mpv != NULL, "mpv_create()");
    if (!mpv)
        return 1;
    mpv_set_option_string(mpv, "vo", "null");
    if (ao[0])
        mpv_set_option_string(mpv, "ao", ao);
    mpv_set_option_string(mpv, "audio-fallback-to-null", "yes");
    mpv_set_option_string(mpv, "config", "no");
    mpv_set_option_string(mpv, "terminal", "no");
    mpv_set_option_string(mpv, "idle", "yes");
    mpv_request_log_messages(mpv, "info");

    // Like media_kit: observed before anything plays, so the core thread
    // reads the list on its own (mp_client_send_property_changes).
    expect(mpv_observe_property(mpv, 1, "audio-device-list", MPV_FORMAT_NODE) >= 0,
           "observe audio-device-list");
    mpv_observe_property(mpv, 2, "playback-time", MPV_FORMAT_DOUBLE);
    expect(mpv_initialize(mpv) >= 0, "mpv_initialize()");

    mpv_node list;
    int n = -1;
    if (mpv_get_property(mpv, "audio-device-list", MPV_FORMAT_NODE, &list) >= 0) {
        n = print_devices("get", &list);
        mpv_free_node_contents(&list);
    }
    // There is always at least "auto".
    expect(n >= 1, "get audio-device-list");

    const char *cmd[] = {"loadfile", file, NULL};
    expect(mpv_command(mpv, cmd) >= 0, "loadfile");

    int observed = 0, loaded = 0, ended = 0;
    double played = 0;
    while (!ended && played < seconds) {
        mpv_event *ev = mpv_wait_event(mpv, 5.0);
        switch (ev->event_id) {
        case MPV_EVENT_NONE:
            printf("(no event for 5 s, playback-time %.1f)\n", played);
            if (!loaded) {
                ended = 1;
            }
            break;
        case MPV_EVENT_LOG_MESSAGE: {
            mpv_event_log_message *m = ev->data;
            printf("[%s] %s: %s", m->level, m->prefix, m->text);
            break;
        }
        case MPV_EVENT_PROPERTY_CHANGE: {
            mpv_event_property *p = ev->data;
            if (ev->reply_userdata == 1 && p->format == MPV_FORMAT_NODE) {
                if (print_devices("observed", p->data) >= 1)
                    observed = 1;
            } else if (ev->reply_userdata == 2 && p->format == MPV_FORMAT_DOUBLE) {
                played = *(double *)p->data;
            }
            break;
        }
        case MPV_EVENT_FILE_LOADED:
            loaded = 1;
            break;
        case MPV_EVENT_END_FILE: {
            mpv_event_end_file *e = ev->data;
            printf("end-file: reason %d, error %d (%s)\n", e->reason, e->error,
                   mpv_error_string(e->error));
            ended = 1;
            break;
        }
        default:
            break;
        }
        fflush(stdout);
    }

    char *cur = mpv_get_property_string(mpv, "current-ao");
    printf("current-ao: %s, played %.1f s\n", cur ? cur : "(none)", played);
    expect(observed, "observed audio-device-list");
    expect(loaded, "file loaded");
    expect(cur != NULL, "an audio output is open");
    expect(played >= seconds - 1.0, "played to the end of the test window");
    mpv_free(cur);

    mpv_terminate_destroy(mpv);
    printf("%s\n", failures ? "FAILED" : "PASSED");
    return failures ? 1 : 0;
}
