import AVFoundation
import AVKit
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
    // PLAY-012: AirPlay for the cast button.
    AirPlayBridge.shared.attach(messenger, view: flutterViewController.view)
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

/// UI-022 (ADR-0117): `kinolune://` links opened in macOS (Info.plist
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

/// PLAY-012 (ADR-0153): AirPlay on the channel `app.kinolune/cast`
/// (lib/cast/cast_platform.dart). media_kit/libmpv cannot send video over
/// AirPlay, so a cast hands the stream (the app's LAN relay of the file or
/// a media server's HLS) to an AVPlayer and routes that player:
///
/// - `startDiscovery`: AVRouteDetector; while it sees AirPlay routes, Dart
///   gets one device, «AirPlay»;
/// - `airPlay`: prepares the AVPlayer at the position, opens the system
///   route picker (AVRoutePickerView for this player) over the cast button,
///   and answers true once the player plays on an AirPlay device, false
///   when the picker closes without one;
/// - `status`, `play`, `pause`, `seek`, `stop` drive that player. If the
///   route goes back to this Mac, the session ends (no picture here).
final class AirPlayBridge: NSObject, AVRoutePickerViewDelegate {
  static let shared = AirPlayBridge()

  private var channel: FlutterMethodChannel?
  private weak var view: NSView?
  private let detector = AVRouteDetector()
  private var routesObserver: NSObjectProtocol?
  private var player: AVPlayer?
  private var picker: AVRoutePickerView?
  private var pending: FlutterResult?
  private var externalObserver: NSKeyValueObservation?
  private var endObserver: NSObjectProtocol?
  private var finished = false
  private var wasExternal = false

  func attach(_ messenger: FlutterBinaryMessenger, view: NSView) {
    self.view = view
    let channel = FlutterMethodChannel(
      name: "app.kinolune/cast", binaryMessenger: messenger)
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self = self else {
        result(FlutterMethodNotImplemented)
        return
      }
      self.handle(call, result)
    }
    self.channel = channel
  }

  private func handle(_ call: FlutterMethodCall, _ result: @escaping FlutterResult) {
    let args = call.arguments as? [String: Any] ?? [:]
    switch call.method {
    case "available":
      result(true)
    case "startDiscovery":
      startDiscovery()
      result(nil)
    case "stopDiscovery":
      detector.isRouteDetectionEnabled = false
      result(nil)
    case "airPlay":
      airPlay(args, result)
    case "status":
      result(status())
    case "play":
      player?.play()
      result(nil)
    case "pause":
      player?.pause()
      result(nil)
    case "seek":
      let ms = (args["positionMs"] as? NSNumber)?.int64Value ?? 0
      player?.seek(to: CMTime(value: ms, timescale: 1000))
      result(nil)
    case "stop":
      end(notify: true)
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  // MARK: discovery

  private func startDiscovery() {
    if routesObserver == nil {
      routesObserver = NotificationCenter.default.addObserver(
        forName: .AVRouteDetectorMultipleRoutesDetectedDidChange,
        object: detector, queue: .main
      ) { [weak self] _ in self?.publish() }
    }
    detector.isRouteDetectionEnabled = true
    publish()
  }

  private func publish() {
    // «Multiple routes»: this Mac and at least one AirPlay device. The name
    // is left to Dart (localized «AirPlay»).
    let devices: [[String: Any]] =
      detector.multipleRoutesDetected
      ? [["id": "airplay", "name": "", "tech": "airPlay"]] : []
    channel?.invokeMethod("devices", arguments: devices)
  }

  // MARK: session

  private func airPlay(_ args: [String: Any], _ result: @escaping FlutterResult) {
    guard let urlString = args["url"] as? String, let url = URL(string: urlString),
      let view = view
    else {
      result(false)
      return
    }
    end(notify: false)
    let player = AVPlayer(url: url)
    player.allowsExternalPlayback = true
    let start = (args["positionMs"] as? NSNumber)?.int64Value ?? 0
    if start > 0 {
      player.seek(to: CMTime(value: start, timescale: 1000))
    }
    self.player = player
    finished = false
    wasExternal = false
    endObserver = NotificationCenter.default.addObserver(
      forName: .AVPlayerItemDidPlayToEndTime, object: player.currentItem, queue: .main
    ) { [weak self] _ in self?.finished = true }
    pending = result
    externalObserver = player.observe(\.isExternalPlaybackActive, options: [.new]) {
      [weak self] player, _ in
      DispatchQueue.main.async { self?.externalChanged(player) }
    }

    // The picker sits over the cast button (Flutter's origin is top-left).
    let x = (args["x"] as? NSNumber)?.doubleValue ?? 0
    let y = (args["y"] as? NSNumber)?.doubleValue ?? 0
    let w = max((args["width"] as? NSNumber)?.doubleValue ?? 1, 1)
    let h = max((args["height"] as? NSNumber)?.doubleValue ?? 1, 1)
    let top = view.isFlipped ? y : view.bounds.height - y - h
    let picker = AVRoutePickerView(frame: NSRect(x: x, y: top, width: w, height: h))
    picker.player = player
    picker.delegate = self
    view.addSubview(picker)
    self.picker = picker
    // Without a button to press, the picker stays over the cast button: the
    // next click opens it.
    Self.firstButton(in: picker)?.performClick(nil)
  }

  private static func firstButton(in view: NSView) -> NSButton? {
    for sub in view.subviews {
      if let b = sub as? NSButton { return b }
      if let b = firstButton(in: sub) { return b }
    }
    return nil
  }

  private func externalChanged(_ player: AVPlayer) {
    guard player === self.player else { return }
    if player.isExternalPlaybackActive {
      wasExternal = true
      player.play()
      answer(true)
    } else if wasExternal {
      // Back to this Mac: the cast is over.
      end(notify: true)
    }
  }

  func routePickerViewDidEndPresentingRoutes(_ routePickerView: AVRoutePickerView) {
    // A route takes a moment to become active after the picker closes.
    DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
      guard let self = self, self.pending != nil,
        self.player?.isExternalPlaybackActive != true
      else { return }
      self.end(notify: false)
    }
  }

  private func answer(_ ok: Bool) {
    let pending = self.pending
    self.pending = nil
    pending?(ok)
    picker?.removeFromSuperview()
    picker = nil
  }

  private func status() -> [String: Any]? {
    guard let player = player, pending == nil else { return nil }
    func ms(_ t: CMTime) -> Int64 {
      t.isNumeric && t.seconds.isFinite ? Int64(t.seconds * 1000) : 0
    }
    var state = "idle"
    var idle = "none"
    if finished {
      idle = "finished"
    } else if player.currentItem?.status == .failed {
      idle = "error"
    } else {
      switch player.timeControlStatus {
      case .playing: state = "playing"
      case .paused: state = "paused"
      case .waitingToPlayAtSpecifiedRate: state = "buffering"
      @unknown default: state = "buffering"
      }
    }
    return [
      "state": state,
      "positionMs": ms(player.currentTime()),
      "durationMs": ms(player.currentItem?.duration ?? .invalid),
      "idleReason": idle,
    ]
  }

  private func end(notify: Bool) {
    if pending != nil { answer(false) }
    externalObserver?.invalidate()
    externalObserver = nil
    if let o = endObserver { NotificationCenter.default.removeObserver(o) }
    endObserver = nil
    let had = player != nil
    player?.pause()
    player?.replaceCurrentItem(with: nil)
    player = nil
    picker?.removeFromSuperview()
    picker = nil
    if notify && had { channel?.invokeMethod("sessionEnded", arguments: nil) }
  }
}
