// SPDX-License-Identifier: MPL-2.0
//
// Deep links and launch arguments (UI-022, ADR-NNNN). One scheme on every
// platform, `kinolune://`:
// * `kinolune://item/<id>` — the page of a library item;
// * `kinolune://play/<id>` — resume a library item (Watch Next cards);
// * `kinolune://channel/<login>` — a Twitch channel's page;
// * `kinolune://twitch/vod/<id>?t=<ms>` — a Twitch VOD at a position;
// * `kinolune://search?q=<text>` — Search with the query.
// A media file path (or `file://` URI) among the arguments plays that file
// ("Open with Kinolune" in the file manager).
//
// Where links come from:
// * Android — intents of the activity (TvBridge, channel `app.kinolune/tv`,
//   android_tv.dart), on phones and TVs;
// * Linux and Windows — the command line: the first start passes it to
//   `main`, a second start hands it to the running window and quits (the
//   runner, channel `app.kinolune/links`);
// * macOS — `application(_:open:)` of the AppDelegate (Info.plist
//   `CFBundleURLTypes`), the same channel.

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../integration/player.dart';
import '../screens/library_item_menu.dart' show playLibraryItem;
import '../screens/playback.dart' show playLooseFile;
import '../screens/routes.dart';
import '../screens/search_screen.dart';
import '../screens/twitch/twitch_channel_screen.dart';
import '../screens/twitch/twitch_play.dart' show playTwitch;
import '../services/app_services.dart';
import '../shell/app_shell.dart';
import '../twitch/twitch_models.dart';
import '../twitch/twitch_service.dart' show TwitchPlayTarget;

const kLinkScheme = 'kinolune'; // l10n-ignore
const _linksChannel = MethodChannel('app.kinolune/links');

/// Twitch logins: 1–25 letters, digits and underscores.
final _twitchLogin = RegExp(r'^[a-zA-Z0-9_]{1,25}$');

/// Audio files play in the music player, everything else in the video one.
const _audioExtensions = {
  'aac', 'aiff', 'alac', 'ape', 'flac', 'm4a', 'm4b', 'mka', 'mp3', //
  'oga', 'ogg', 'opus', 'wav', 'wma',
};

/// A link or argument the app was opened with.
sealed class AppLink {
  const AppLink();

  /// Parses a `kinolune://` link; null for anything else.
  static AppLink? parse(String link) {
    final u = Uri.tryParse(link.trim());
    if (u == null || u.scheme != kLinkScheme) return null;
    final seg = u.pathSegments.where((s) => s.isNotEmpty).toList();
    switch (u.host) {
      case 'play' when seg.length == 1:
        return PlayLink(seg.first);
      case 'item' when seg.length == 1:
        return ItemLink(seg.first);
      case 'channel' when seg.length == 1 && _twitchLogin.hasMatch(seg.first):
        return ChannelLink(seg.first.toLowerCase());
      case 'twitch' when seg.length == 2 && seg.first == 'vod':
        final t = int.tryParse(u.queryParameters['t'] ?? '');
        return TwitchVodLink(seg[1], startMs: t);
      case 'search' when seg.isEmpty:
        final q = (u.queryParameters['q'] ?? '').trim();
        return q.isEmpty ? null : SearchLink(q);
    }
    return null;
  }

  /// One command-line argument: a `kinolune://` link, a `file://` URI or an
  /// absolute path of a file; null for options and anything else.
  static AppLink? fromArgument(String arg) {
    final a = arg.trim();
    if (a.isEmpty || a.startsWith('-')) return null;
    if (a.startsWith('$kLinkScheme:')) return parse(a);
    String? path;
    if (a.startsWith('file:')) {
      try {
        path = Uri.parse(a).toFilePath();
      } on UnsupportedError {
        return null;
      } on FormatException {
        return null;
      }
    } else if (a.startsWith('/') || RegExp(r'^[a-zA-Z]:[\\/]').hasMatch(a)) {
      path = a;
    }
    if (path == null) return null;
    final file = FileLink(path);
    // A folder or a program, not a media file.
    return file.name.contains('.') ? file : null;
  }

  /// Every link among [args], in order.
  static List<AppLink> fromArguments(Iterable<String> args) => [
    for (final a in args) ?fromArgument(a),
  ];

  /// The link resuming library item [id].
  static String play(String id) =>
      Uri(scheme: kLinkScheme, host: 'play', pathSegments: [id]).toString();

  /// The link to the page of library item [id].
  static String item(String id) =>
      Uri(scheme: kLinkScheme, host: 'item', pathSegments: [id]).toString();

  /// The link to Twitch channel [login].
  static String channel(String login) => Uri(
    scheme: kLinkScheme,
    host: 'channel',
    pathSegments: [login],
  ).toString();

  /// The link to Search with [query].
  static String search(String query) => Uri(
    scheme: kLinkScheme,
    host: 'search',
    queryParameters: {'q': query},
  ).toString();

  /// The link resuming Twitch VOD [videoId] at [startMs].
  static String twitchVod(String videoId, int startMs) => Uri(
    scheme: kLinkScheme,
    host: 'twitch',
    pathSegments: ['vod', videoId],
    queryParameters: {'t': '$startMs'},
  ).toString();
}

class PlayLink extends AppLink {
  const PlayLink(this.id);
  final String id;
}

class ItemLink extends AppLink {
  const ItemLink(this.id);
  final String id;
}

class ChannelLink extends AppLink {
  const ChannelLink(this.login);
  final String login;
}

class TwitchVodLink extends AppLink {
  const TwitchVodLink(this.videoId, {this.startMs});
  final String videoId;
  final int? startMs;
}

class SearchLink extends AppLink {
  const SearchLink(this.query);
  final String query;
}

/// A media file to play as it is (outside the library).
class FileLink extends AppLink {
  const FileLink(this.path);
  final String path;

  String get name => path.split(RegExp(r'[\\/]')).last;

  bool get audio => _audioExtensions.contains(
    name.contains('.') ? name.split('.').last.toLowerCase() : '',
  );
}

/// Opens [link] in [shell]: a page or the player over Home, Search with the
/// query, a channel over the current section.
Future<void> openAppLink(AppShellState shell, AppLink link) async {
  // A link of the cold start comes before the shell's first frame is done.
  await WidgetsBinding.instance.endOfFrame;
  if (!shell.mounted) return;
  final services = Services.read(shell.context);
  NavigatorState? homeNav() {
    if (!shell.mounted) return null;
    if (shell.section != Section.home) shell.select(Section.home);
    final nav = shell.navigatorOf(Section.home);
    nav?.popUntil((r) => r.isFirst);
    return nav;
  }

  switch (link) {
    case SearchLink(:final query):
      shell.select(Section.search);
      final nav = shell.navigatorOf(Section.search);
      nav?.popUntil((r) => r.isFirst);
      unawaited(
        nav?.push(
          MaterialPageRoute<void>(
            builder: (_) => SearchScreen(initialQuery: query),
          ),
        ),
      );
    case ChannelLink(:final login):
      // Twitch hidden (Settings, a restricted profile): no channel pages.
      if (!services.twitchVisible) {
        debugPrint('Channel link while the Twitch section is hidden');
        return;
      }
      final nav = homeNav();
      if (nav == null) return;
      final favorite = services.twitch.favorites
          .where((f) => f.login == login)
          .firstOrNull;
      unawaited(
        nav.push(
          MaterialPageRoute<void>(
            builder: (_) => TwitchChannelScreen(
              channel: TwitchChannel(
                login: login,
                displayName: favorite?.displayName ?? login,
                userId: favorite?.userId,
                avatarUrl: favorite?.avatarUrl,
              ),
            ),
          ),
        ),
      );
    case TwitchVodLink(:final videoId, :final startMs):
      final nav = homeNav();
      if (nav == null) return;
      await playTwitch(
        nav.context,
        TwitchPlayTarget.vod(
          videoId,
          startAt: startMs == null ? null : Duration(milliseconds: startMs),
        ),
      );
    case FileLink():
      final nav = homeNav();
      final ctx = nav?.context;
      if (ctx == null || !ctx.mounted) return;
      await playLooseFile(
        ctx,
        PlayRequest(uri: link.path, title: link.name),
        audio: link.audio,
      );
    case PlayLink(:final id) || ItemLink(:final id):
      final item = await services.library.item(id);
      if (item == null) {
        debugPrint('Link to an unknown item');
        return;
      }
      final showId = item.showId;
      final show = item.kind == ItemKind.episode && showId != null
          ? await services.library.item(showId)
          : null;
      final nav = homeNav();
      final ctx = nav?.context;
      if (ctx == null || !ctx.mounted) return;
      if (link is PlayLink) {
        await playLibraryItem(ctx, item);
      } else {
        openItem(ctx, item, show: show);
      }
  }
}

/// Links of the desktop: the arguments of the first start ([initial], set by
/// `main`) once per process, then those the runner hands over from a second
/// start or a URL open (channel `app.kinolune/links`: `take` returns what
/// came before Dart listened, `open` brings new ones).
class LaunchLinks {
  LaunchLinks({required this.onLink, this.channel = _linksChannel});

  /// Arguments of the first start (`main(args)`).
  static List<String> initial = const [];
  static bool _initialTaken = false;

  /// Lets a test start over.
  @visibleForTesting
  static void reset({List<String> initial = const []}) {
    LaunchLinks.initial = initial;
    _initialTaken = false;
  }

  final Future<void> Function(AppLink link) onLink;
  final MethodChannel channel;
  bool _disposed = false;

  Future<void> start() async {
    channel.setMethodCallHandler(_onCall);
    if (!_initialTaken) {
      _initialTaken = true;
      _deliver(initial);
    }
    try {
      final pending = await channel.invokeListMethod<String>('take');
      if (pending != null && !_disposed) _deliver(pending);
    } on MissingPluginException {
      return;
    } on PlatformException catch (e) {
      debugPrint('Launch links unavailable: $e');
    }
  }

  Future<Object?> _onCall(MethodCall call) async {
    if (call.method == 'open' && call.arguments is List && !_disposed) {
      _deliver([
        for (final a in call.arguments as List)
          if (a is String) a,
      ]);
      return true;
    }
    throw MissingPluginException();
  }

  void _deliver(List<String> args) {
    for (final link in AppLink.fromArguments(args)) {
      unawaited(onLink(link));
    }
  }

  void dispose() {
    _disposed = true;
    channel.setMethodCallHandler(null);
  }
}

/// Whether this platform gets its links through [LaunchLinks].
bool get launchLinksPlatform =>
    !kIsWeb && (Platform.isLinux || Platform.isWindows || Platform.isMacOS);
