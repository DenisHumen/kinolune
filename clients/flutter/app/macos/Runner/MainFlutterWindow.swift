import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)

    // UI-022: kinolune:// links for Dart and the menu bar's "Close Window".
    let messenger = flutterViewController.engine.binaryMessenger
    LaunchLinks.shared.attach(messenger, window: self)
    let windowChannel = FlutterMethodChannel(
      name: "app.kinolune/window", binaryMessenger: messenger)
    windowChannel.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "close":
        self?.performClose(nil)
        result(nil)
      default:
        result(FlutterMethodNotImplemented)
      }
    }

    super.awakeFromNib()
  }
}

/// UI-022 (ADR-NNNN): `kinolune://` links opened in macOS (Info.plist
/// `CFBundleURLTypes`) go to Dart on the channel `app.kinolune/links`
/// (lib/platform/app_links.dart): `open` with the links while Dart listens,
/// `take` for those that came before. LaunchServices keeps one copy of the
/// app running, so a link brings the open window to the front.
final class LaunchLinks: NSObject {
  static let shared = LaunchLinks()

  private var channel: FlutterMethodChannel?
  private weak var window: NSWindow?
  private var pending: [String] = []

  func attach(_ messenger: FlutterBinaryMessenger, window: NSWindow) {
    self.window = window
    let channel = FlutterMethodChannel(
      name: "app.kinolune/links", binaryMessenger: messenger)
    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "take", let self = self else {
        result(FlutterMethodNotImplemented)
        return
      }
      result(self.pending)
      self.pending.removeAll()
    }
    self.channel = channel
    // The Apple Event of a URL open; installed before launching finishes,
    // so a link that starts the app is not lost.
    NSAppleEventManager.shared().setEventHandler(
      self,
      andSelector: #selector(handleGetURL(_:withReply:)),
      forEventClass: AEEventClass(kInternetEventClass),
      andEventID: AEEventID(kAEGetURL))
  }

  @objc private func handleGetURL(
    _ event: NSAppleEventDescriptor, withReply reply: NSAppleEventDescriptor
  ) {
    guard
      let link = event.paramDescriptor(forKeyword: AEKeyword(keyDirectObject))?
        .stringValue
    else { return }
    window?.makeKeyAndOrderFront(nil)
    open([link])
  }

  func open(_ args: [String]) {
    guard !args.isEmpty else { return }
    guard let channel = channel else {
      pending.append(contentsOf: args)
      return
    }
    // Dart answers true once it took them; otherwise they wait for `take`.
    channel.invokeMethod("open", arguments: args) { [weak self] result in
      if (result as? Bool) != true {
        self?.pending.append(contentsOf: args)
      }
    }
  }
}
