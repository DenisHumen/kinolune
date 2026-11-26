// SPDX-License-Identifier: MPL-2.0
import 'dart:async';

import 'package:flutter/foundation.dart';

/// Side panels of the OSD. [audio], [subtitles] and [video] are the tabs
/// of one track panel (UI-014).
enum OsdPanel {
  none,
  audio,
  subtitles,
  video,
  subtitleFile,
  subtitleSearch,
  speed,
  zoom,
  chapters,
  info,

  /// The sleep timer (PLAY-022).
  sleepTimer,
}

/// Where an OSD panel opens (UI-014): next to the picture on a desktop or
/// tablet, as a bottom sheet on a phone, across the top on a TV (tabs a
/// remote reaches with Up and Left/Right).
enum OsdPanelPlacement { side, bottom, top }

extension OsdPanelTabs on OsdPanel {
  /// One of the Audio · Subtitles · Video tabs.
  bool get isTrackTab =>
      this == OsdPanel.audio ||
      this == OsdPanel.subtitles ||
      this == OsdPanel.video;

  /// A page opened from the subtitles tab ("Find subtitles", a file):
  /// Back returns to the tab.
  bool get isSubtitlePage =>
      this == OsdPanel.subtitleSearch || this == OsdPanel.subtitleFile;
}

/// Visibility logic of the player OSD: shown on interaction, auto-hidden
/// after [autoHide] while playing; stays up while paused or a panel is open.
class OsdController extends ChangeNotifier {
  OsdController({this.autoHide = const Duration(seconds: 4)});

  final Duration autoHide;
  bool _visible = true;
  bool _playing = false;
  OsdPanel _panel = OsdPanel.none;
  Timer? _timer;

  bool get visible => _visible;

  /// Keeps the OSD up while a screen reader is on (UI-021): its controls
  /// must stay reachable by swiping, so the auto-hide timer does not run.
  bool get holdVisible => _holdVisible;
  bool _holdVisible = false;
  set holdVisible(bool v) {
    if (v == _holdVisible) return;
    _holdVisible = v;
    if (v) {
      show();
    } else {
      _restartTimer();
    }
  }

  OsdPanel get panel => _panel;

  /// Shows the OSD and restarts the auto-hide timer.
  void show() {
    final changed = !_visible;
    _visible = true;
    _restartTimer();
    if (changed) notifyListeners();
  }

  void hide() {
    if (_panel != OsdPanel.none || _holdVisible) return;
    _timer?.cancel();
    if (_visible) {
      _visible = false;
      notifyListeners();
    }
  }

  void toggle() => _visible ? hide() : show();

  /// Feed the engine's playing flag.
  set playing(bool v) {
    if (v == _playing) return;
    _playing = v;
    if (!v) {
      show();
    } else {
      _restartTimer();
    }
  }

  void openPanel(OsdPanel p) {
    _panel = p;
    _visible = true;
    _timer?.cancel();
    notifyListeners();
    if (p == OsdPanel.none) _restartTimer();
  }

  void closePanel() => openPanel(OsdPanel.none);

  void _restartTimer() {
    _timer?.cancel();
    if (!_playing || _panel != OsdPanel.none || _holdVisible) return;
    _timer = Timer(autoHide, hide);
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }
}
