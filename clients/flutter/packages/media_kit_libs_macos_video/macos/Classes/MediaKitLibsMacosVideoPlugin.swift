// SPDX-License-Identifier: MPL-2.0
//
// Nothing to register: the package only carries the frameworks
// (Mpv.framework and FFmpeg) that the app embeds.
import Cocoa
import FlutterMacOS

public class MediaKitLibsMacosVideoPlugin: NSObject, FlutterPlugin {
  public static func register(with registrar: FlutterPluginRegistrar) {}
}
