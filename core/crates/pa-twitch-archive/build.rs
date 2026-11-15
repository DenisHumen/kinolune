// SPDX-License-Identifier: MPL-2.0
//! Build flavor cfgs (`cfg(pa_twitch_archive)`) from `pa-flavor`
//! (CORE-013): the whole crate is full-only.

fn main() {
    pa_flavor::build::emit_cfgs();
}
