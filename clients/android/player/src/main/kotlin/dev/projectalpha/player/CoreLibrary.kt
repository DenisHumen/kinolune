// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

/**
 * Which copy of the core the UniFFI bindings use (PLAY-AND-003, ADR-0127).
 *
 * The bindings load `libpa_ffi.so` by default. In the Flutter app the core already runs as `libpa_frb.so`
 * (flutter_rust_bridge), which links pa-ffi and exports all of its UniFFI symbols. Two copies of the core in one
 * process do not share state: the loopback proxy the app's library started lives in `libpa_frb.so`, so Media3 can
 * read its files in-process ([CoreDataSource]) only when the bindings use that copy too.
 */
object CoreLibrary {
    /** UniFFI's system property naming the library of the `pa_ffi` component. */
    const val OVERRIDE_PROPERTY = "uniffi.component.pa_ffi.libraryOverride"

    /** The app's core (`libpa_frb.so`). */
    const val APP_CORE = "pa_frb"

    /**
     * Binds the bindings to the app's core when this process has it. Call before the first core call (the bindings
     * pick their library once per process). Returns whether the bindings use the app's core. Without it, the
     * bindings keep `libpa_ffi.so`, and [CoreDataSource] reads the app's files through the proxy over HTTP.
     */
    @Synchronized
    fun preferAppCore(): Boolean {
        System.getProperty(OVERRIDE_PROPERTY)?.let { return it == APP_CORE }
        return try {
            System.loadLibrary(APP_CORE)
            System.setProperty(OVERRIDE_PROPERTY, APP_CORE)
            true
        } catch (_: UnsatisfiedLinkError) {
            false
        }
    }
}
