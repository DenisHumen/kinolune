// SPDX-License-Identifier: MPL-2.0
//! Build flavor cfgs (`cfg(pa_twitch_native)`) from `pa-flavor` (CORE-013):
//! the native Twitch playback API (TWI-014) has bodies only in full builds.

fn main() {
    pa_flavor::build::emit_cfgs();
}
