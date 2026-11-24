// SPDX-License-Identifier: MPL-2.0
//
// The Twitch chat list (TWI-012, research 06 §3.3–3.4).
//
// * Virtualised: a reversed ListView.builder over the controller's ring
//   buffer; only visible lines are built, and a line's widget is reused
//   until its moderation state or the image set changes, so a batch of new
//   lines does not rebuild the old ones.
// * Emotes and badges are images the core downloaded once per key; every
//   use of a key shares one ImageProvider, so Flutter decodes an animated
//   emote once and all its copies play the same frames. "Pause animations"
//   switches tickers off (TickerMode): animated emotes stand still on
//   their current frame and cost no CPU.
// * Name colours are corrected for contrast on the chat background.
// * Scrolled up: the list stays still and a "N new messages" button
//   returns to the newest line.
// * TV density: the body font of the TV tokens (29 px on a 1080p screen),
//   readable from 3 m.
// * TWI-024: lines that mention the user are tinted; a long press (or a
//   right click) on a line opens its actions (mention, block).

import 'dart:io';

import 'package:material_ui/material_ui.dart';
import 'package:pa_design/pa_design.dart';

import '../../../l10n/l10n.dart';
import '../../../theme/kinolune_theme.dart';
import 'chat_controller.dart';
import 'chat_model.dart';
import 'name_color.dart';

/// Sizes and colours of the chat for one density.
@immutable
class ChatStyle {
  const ChatStyle({
    required this.fontSize,
    required this.background,
    required this.text,
    required this.secondary,
    required this.lineGap,
    required this.padding,
  });

  factory ChatStyle.of(BuildContext context) {
    final d = KDensity.of(context);
    final c = PaColorScheme.dark;
    final font = d.type.body;
    return ChatStyle(
      fontSize: font,
      background: c.bgElevated,
      text: c.textPrimary,
      secondary: c.textSecondary,
      lineGap: d.isTv ? font * .35 : font * .25,
      padding: EdgeInsets.symmetric(horizontal: d.isTv ? font : font * .75),
    );
  }

  final double fontSize;
  final Color background;
  final Color text;
  final Color secondary;
  final double lineGap;
  final EdgeInsets padding;

  /// Emotes are about twice the text height (Twitch: 28 px on 13 px).
  double get emoteHeight => fontSize * 1.9;
  double get badgeSize => fontSize * 1.25;

  TextStyle get body => TextStyle(
    fontFamily: kFontFamily,
    fontFamilyFallback: kFontFallback,
    fontSize: fontSize,
    height: 1.35,
    color: text,
  );
}

/// The chat list of [controller].
class TwitchChatView extends StatefulWidget {
  const TwitchChatView({
    super.key,
    required this.controller,
    this.pauseAnimations = false,
    this.onLineAction,
  });

  final ChatController controller;

  /// Long press or right click on a user's line (TWI-024: mention, block).
  final void Function(ChatLine line)? onLineAction;

  /// Animated emotes stand still (setting, or the system's reduce motion).
  final bool pauseAnimations;

  @override
  State<TwitchChatView> createState() => _TwitchChatViewState();
}

/// Distance from the newest line that counts as "scrolled up".
const _pauseOffset = 48.0;

class _TwitchChatViewState extends State<TwitchChatView> {
  final _scroll = ScrollController();
  final _providers = <String, ImageProvider>{};
  final _built = Expando<_BuiltLine>();
  NameColors? _names;
  ChatStyle? _style;
  double _dpr = 1;

  ChatController get _c => widget.controller;

  @override
  void initState() {
    super.initState();
    _c.addListener(_changed);
    _scroll.addListener(_scrolled);
  }

  @override
  void didUpdateWidget(TwitchChatView old) {
    super.didUpdateWidget(old);
    if (old.controller != widget.controller) {
      old.controller.removeListener(_changed);
      _c.addListener(_changed);
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final style = ChatStyle.of(context);
    final dpr = MediaQuery.devicePixelRatioOf(context);
    if (style.fontSize != _style?.fontSize || dpr != _dpr) {
      _providers.clear();
    }
    _style = style;
    _dpr = dpr;
    _names = NameColors(style.background);
  }

  @override
  void dispose() {
    _c.removeListener(_changed);
    _scroll.dispose();
    super.dispose();
  }

  void _changed() => setState(() {});

  void _scrolled() {
    final o = _scroll.offset;
    if (o > _pauseOffset) {
      _c.pause();
    } else if (o <= 1 && !_c.following) {
      _c.resume();
    }
  }

  void _toNewest() {
    if (_scroll.hasClients) _scroll.jumpTo(0);
    _c.resume();
  }

  /// One provider per image key and size: one decode, shared by all uses.
  ImageProvider? _provider(String key, double height) {
    final img = _c.image(key);
    if (img == null) return null;
    return _providers[key] ??= ResizeImage(
      FileImage(File(img.path)),
      height: (height * _dpr).round(),
      policy: ResizeImagePolicy.fit,
    );
  }

  @override
  Widget build(BuildContext context) {
    final style = _style!;
    final l = context.l10n;
    final count = _c.length;
    Widget list;
    if (count == 0) {
      list = Center(
        child: Padding(
          padding: style.padding,
          child: Text(
            switch (_c.connection) {
              ChatConnection.connecting => l.twitchChatConnecting,
              ChatConnection.connected => l.twitchChatWaiting,
              ChatConnection.reconnecting => l.twitchChatReconnecting,
              ChatConnection.stopped => l.twitchChatDisconnected,
            },
            textAlign: TextAlign.center,
            style: style.body.copyWith(color: style.secondary),
          ),
        ),
      );
    } else {
      list = ListView.builder(
        controller: _scroll,
        reverse: true,
        padding: EdgeInsets.symmetric(vertical: style.lineGap),
        itemCount: count,
        findChildIndexCallback: (key) {
          // Lines keep their element (and layout) when new ones arrive.
          final id = (key as ValueKey<String>).value;
          for (var i = 0; i < count && i < 200; i++) {
            if (_c[count - 1 - i].id == id) return i;
          }
          return null;
        },
        itemBuilder: (context, i) => _line(_c[count - 1 - i], style),
      );
    }
    final banner = switch (_c.connection) {
      ChatConnection.reconnecting when count > 0 => l.twitchChatReconnecting,
      ChatConnection.stopped when count > 0 => l.twitchChatDisconnected,
      _ => null,
    };
    return ColoredBox(
      color: style.background,
      child: TickerMode(
        enabled: !widget.pauseAnimations,
        child: Stack(
          children: [
            Positioned.fill(child: list),
            if (banner != null)
              Positioned(
                top: 0,
                left: 0,
                right: 0,
                child: _Banner(text: banner, style: style),
              ),
            if (_c.skipped > 0)
              Positioned(
                top: style.lineGap,
                right: style.padding.right,
                child: _Pill(text: '+${_c.skipped}', style: style),
              ),
            if (!_c.following && _c.unseen > 0)
              Positioned(
                bottom: style.fontSize,
                left: 0,
                right: 0,
                child: Center(
                  child: _NewMessages(
                    text: l.twitchChatNewMessages(_c.unseen),
                    style: style,
                    onPressed: _toNewest,
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }

  Widget _line(ChatLine line, ChatStyle style) {
    final built = _built[line];
    if (built != null &&
        built.fate == line.fate &&
        built.images == _c.imagesVersion &&
        identical(built.style, style)) {
      return built.widget;
    }
    Widget content = Padding(
      padding: style.padding.add(
        EdgeInsets.symmetric(vertical: style.lineGap / 2),
      ),
      child: line.kind == ChatLineKind.system
          ? _systemLine(line, style)
          : _messageLine(line, style),
    );
    if (line.kind == ChatLineKind.message && line.login.isNotEmpty) {
      void action() => widget.onLineAction?.call(line);
      content = GestureDetector(
        behavior: HitTestBehavior.opaque,
        onLongPress: action,
        onSecondaryTap: action,
        child: content,
      );
    }
    final w = KeyedSubtree(key: ValueKey(line.id), child: content);
    _built[line] = _BuiltLine(w, line.fate, _c.imagesVersion, style);
    return w;
  }

  Widget _systemLine(ChatLine line, ChatStyle style) {
    final l = context.l10n;
    final text = switch (line.system!) {
      ChatClearedText() => l.twitchChatCleared,
      UserTimedOutText(:final login, :final seconds) => l.twitchChatTimedOut(
        login,
        seconds,
      ),
      UserBannedText(:final login) => l.twitchChatBanned(login),
      ServerNoticeText(:final text) => text,
    };
    return Text(
      text,
      style: style.body.copyWith(
        color: style.secondary,
        fontStyle: FontStyle.italic,
      ),
    );
  }

  Widget _messageLine(ChatLine line, ChatStyle style) {
    final l = context.l10n;
    final nameColor = _names!.of(line.color, line.login);
    final removed = line.fate != ChatLineFate.shown;
    final spans = <InlineSpan>[
      for (final b in line.badges)
        if (_badge(b, style) case final w?)
          WidgetSpan(
            alignment: PlaceholderAlignment.middle,
            child: Padding(
              padding: EdgeInsetsDirectional.only(end: style.fontSize * .25),
              child: w,
            ),
          ),
      TextSpan(
        text: line.shownName,
        style: TextStyle(color: nameColor, fontWeight: FontWeight.w700),
      ),
      TextSpan(text: line.isAction ? ' ' : ': '),
    ];
    if (removed) {
      spans.add(
        TextSpan(
          text: l.twitchChatMessageDeleted,
          style: TextStyle(color: style.secondary, fontStyle: FontStyle.italic),
        ),
      );
    } else {
      final bodyStyle = line.isAction
          ? TextStyle(color: nameColor, fontStyle: FontStyle.italic)
          : null;
      // Images of the emote just drawn and its zero-width overlays
      // (TWI-023): they share one place, overlays on top.
      List<Widget>? group;
      for (final f in line.fragments) {
        final key = f.emoteKey;
        final provider = key == null ? null : _provider(key, style.emoteHeight);
        if (provider == null) {
          // An overlay whose image is not here yet: its code after a space.
          final text = f.overlay ? ' ${f.text}' : f.text;
          spans.add(TextSpan(text: text, style: bodyStyle));
          group = null;
          continue;
        }
        final image = Image(
          image: provider,
          height: style.emoteHeight,
          gaplessPlayback: true,
          filterQuality: FilterQuality.medium,
          semanticLabel: f.text,
          excludeFromSemantics: false,
        );
        final base = group;
        if (f.overlay && base != null) {
          base.add(image);
          spans[spans.length - 1] = WidgetSpan(
            alignment: PlaceholderAlignment.middle,
            child: Stack(
              alignment: Alignment.center,
              clipBehavior: Clip.none,
              children: List.of(base),
            ),
          );
        } else {
          group = [image];
          spans.add(
            WidgetSpan(alignment: PlaceholderAlignment.middle, child: image),
          );
        }
      }
    }
    Widget body = Text.rich(TextSpan(children: spans), style: style.body);
    final notice = line.noticeText;
    final reply = line.replyTo;
    final mention = line.mention && !removed;
    if (notice != null || reply != null || line.highlighted || mention) {
      body = Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          if (notice != null)
            Text(
              notice,
              style: style.body.copyWith(fontWeight: FontWeight.w600),
            ),
          if (reply != null)
            Text(
              l.twitchChatReplyingTo(reply),
              style: style.body.copyWith(
                color: style.secondary,
                fontSize: style.fontSize * .85,
              ),
            ),
          body,
        ],
      );
      if (notice != null || line.highlighted || mention) {
        // A mention of the user stands out from announcements.
        final tint = mention ? KColors.live : KColors.accent;
        body = DecoratedBox(
          key: mention ? const ValueKey('twitch-chat-mention') : null,
          decoration: BoxDecoration(
            color: tint.withValues(alpha: .18),
            border: BorderDirectional(
              start: BorderSide(
                color: mention ? KColors.live : KColors.accentSoft,
                width: 3,
              ),
            ),
          ),
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: style.fontSize * .5,
              vertical: style.lineGap,
            ),
            child: body,
          ),
        );
      }
    }
    return body;
  }

  Widget? _badge(ChatBadge b, ChatStyle style) {
    final size = style.badgeSize;
    final provider = b.imageKey.isEmpty ? null : _provider(b.imageKey, size);
    if (provider != null) {
      return Image(
        image: provider,
        width: size,
        height: size,
        semanticLabel: b.title.isEmpty ? null : b.title,
      );
    }
    final mark = _marks[b.setId];
    if (mark == null) return null;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: Color(mark.$1),
        borderRadius: BorderRadius.circular(size * .2),
      ),
      child: Icon(mark.$2, size: size * .75, color: const Color(0xFFFFFFFF)),
    );
  }
}

/// Marks for badges without an image (anonymous chat): colour and icon.
const _marks = <String, (int, IconData)>{
  'broadcaster': (0xFFE91916, Icons.videocam_rounded),
  'moderator': (0xFF00AD03, Icons.shield_rounded),
  'vip': (0xFFE005B9, Icons.diamond_rounded),
  'subscriber': (0xFF8205B4, Icons.star_rounded),
  'founder': (0xFF8205B4, Icons.star_outline_rounded),
  'partner': (0xFF9146FF, Icons.verified_rounded),
  'staff': (0xFF2D2D2D, Icons.build_rounded),
};

class _BuiltLine {
  _BuiltLine(this.widget, this.fate, this.images, this.style);
  final Widget widget;
  final ChatLineFate fate;
  final int images;
  final ChatStyle style;
}

class _Banner extends StatelessWidget {
  const _Banner({required this.text, required this.style});
  final String text;
  final ChatStyle style;

  @override
  Widget build(BuildContext context) => ColoredBox(
    color: KColors.scrimStrong,
    child: Padding(
      padding: style.padding.add(EdgeInsets.symmetric(vertical: style.lineGap)),
      child: Text(text, style: style.body.copyWith(color: style.secondary)),
    ),
  );
}

class _Pill extends StatelessWidget {
  const _Pill({required this.text, required this.style});
  final String text;
  final ChatStyle style;

  @override
  Widget build(BuildContext context) => DecoratedBox(
    decoration: BoxDecoration(
      color: KColors.bgElevated2,
      borderRadius: BorderRadius.circular(style.fontSize),
    ),
    child: Padding(
      padding: EdgeInsets.symmetric(
        horizontal: style.fontSize * .6,
        vertical: style.fontSize * .2,
      ),
      child: Text(
        text,
        style: style.body.copyWith(
          color: style.secondary,
          fontSize: style.fontSize * .85,
        ),
      ),
    ),
  );
}

class _NewMessages extends StatelessWidget {
  const _NewMessages({
    required this.text,
    required this.style,
    required this.onPressed,
  });
  final String text;
  final ChatStyle style;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => Material(
    color: KColors.accent,
    shape: const StadiumBorder(),
    child: InkWell(
      customBorder: const StadiumBorder(),
      onTap: onPressed,
      child: Padding(
        padding: EdgeInsets.symmetric(
          horizontal: style.fontSize,
          vertical: style.fontSize * .45,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              Icons.arrow_downward_rounded,
              size: style.fontSize * 1.1,
              color: PaTokens.textOnAccent,
            ),
            SizedBox(width: style.fontSize * .4),
            Text(
              text,
              style: style.body.copyWith(
                color: PaTokens.textOnAccent,
                fontWeight: FontWeight.w600,
              ),
            ),
          ],
        ),
      ),
    ),
  );
}
