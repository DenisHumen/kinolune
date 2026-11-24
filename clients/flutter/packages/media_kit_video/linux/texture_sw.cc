// This file is a part of media_kit
// (https://github.com/media-kit/media-kit).
//
// Copyright © 2021 & onwards, Hitesh Kumar Saini <saini123hitesh@gmail.com>.
// All rights reserved.
// Use of this source code is governed by MIT license that can be found in the
// LICENSE file.

#include "include/media_kit_video/texture_sw.h"

// Kinolune (BUG-001): FlPixelBufferTexture (Flutter) uploads the frames to a
// GL texture it creates on the raster thread, in Flutter's context, and
// deletes it in dispose. TextureSW was disposed on the platform thread, where
// that context is not current, so the texture (a full frame, up to 8 MB)
// stayed alive for every closed player. The last reference is dropped on the
// raster thread instead: the next copy_pixels of any TextureSW.
static GMutex pending_unref_mutex;
static GPtrArray* pending_unref = NULL;

void texture_sw_unref_later(TextureSW* self) {
  g_mutex_lock(&pending_unref_mutex);
  if (pending_unref == NULL) {
    pending_unref = g_ptr_array_new();
  }
  g_ptr_array_add(pending_unref, self);
  g_mutex_unlock(&pending_unref_mutex);
}

// Must run with Flutter's context current (the raster thread in populate).
static void unref_pending_textures() {
  GPtrArray* textures = NULL;
  g_mutex_lock(&pending_unref_mutex);
  if (pending_unref != NULL && pending_unref->len > 0) {
    textures = pending_unref;
    pending_unref = NULL;
  }
  g_mutex_unlock(&pending_unref_mutex);
  if (textures != NULL) {
    for (guint i = 0; i < textures->len; i++) {
      g_object_unref(g_ptr_array_index(textures, i));
    }
    g_ptr_array_free(textures, TRUE);
  }
}

struct _TextureSW {
  FlPixelBufferTexture parent_instance;
  guint32 current_width;
  guint32 current_height;
  VideoOutput* video_output;
};

G_DEFINE_TYPE(TextureSW, texture_sw, fl_pixel_buffer_texture_get_type())

static void texture_sw_init(TextureSW* self) {
  self->current_width = 1;
  self->current_height = 1;
  self->video_output = NULL;
}

static void texture_sw_dispose(GObject* object) {
  G_OBJECT_CLASS(texture_sw_parent_class)->dispose(object);
}

static void texture_sw_class_init(TextureSWClass* klass) {
  FL_PIXEL_BUFFER_TEXTURE_CLASS(klass)->copy_pixels = texture_sw_copy_pixels;
  G_OBJECT_CLASS(klass)->dispose = texture_sw_dispose;
}

TextureSW* texture_sw_new(VideoOutput* video_output) {
  TextureSW* self = TEXTURE_SW(g_object_new(texture_sw_get_type(), NULL));
  self->video_output = video_output;
  return self;
}

gboolean texture_sw_copy_pixels(FlPixelBufferTexture* texture,
                                const guint8** buffer,
                                guint32* width,
                                guint32* height,
                                GError** error) {
  // Flutter's context is current here: free textures of closed videos.
  unref_pending_textures();
  TextureSW* self = TEXTURE_SW(texture);
  VideoOutput* video_output = self->video_output;
  gint32 required_width = (guint32)video_output_get_width(video_output);
  gint32 required_height = (guint32)video_output_get_height(video_output);
  if (required_width > 0 && required_height > 0) {
    guint8* pixel_buffer = video_output_get_pixel_buffer(video_output);
    if (self->current_width != required_width ||
        self->current_height != required_height) {
      self->current_width = required_width;
      self->current_height = required_height;
      // Notify Flutter about the change in texture's dimensions.
      video_output_notify_texture_update(video_output);
    }
    *buffer = pixel_buffer;
    *width = required_width;
    *height = required_height;
  }
  return TRUE;
}
