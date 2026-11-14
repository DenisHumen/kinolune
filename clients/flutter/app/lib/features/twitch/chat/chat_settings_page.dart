// SPDX-License-Identifier: MPL-2.0
//
// Chat settings of the active profile (TWI-024): hide bots, highlight my
// name, highlight words, blocked users, word filter (plain or regex; hide
// or mask). Every change is saved at once through [onChanged].

import 'package:material_ui/material_ui.dart';

import '../../../l10n/l10n.dart';
import '../../../theme/kinolune_theme.dart';
import 'chat_settings.dart';

/// Checks a rule; returns why it is invalid or null. The app passes the
/// core's check (the same regex engine that filters the chat).
typedef ChatRuleCheck = String? Function(ChatWordRule rule);

class TwitchChatSettingsPage extends StatefulWidget {
  const TwitchChatSettingsPage({
    super.key,
    required this.settings,
    required this.onChanged,
    this.checkRule,
    this.skippedRules = 0,
  });

  final ChatSettings settings;
  final ValueChanged<ChatSettings> onChanged;
  final ChatRuleCheck? checkRule;

  /// Rules the running chat skipped as invalid.
  final int skippedRules;

  @override
  State<TwitchChatSettingsPage> createState() => _TwitchChatSettingsPageState();
}

class _TwitchChatSettingsPageState extends State<TwitchChatSettingsPage> {
  late ChatSettings _s = widget.settings;

  void _set(ChatSettings s) {
    setState(() => _s = s);
    widget.onChanged(s);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final caption = TextStyle(color: KColors.textSecondary);
    return Scaffold(
      backgroundColor: KColors.bgBase,
      appBar: AppBar(
        backgroundColor: KColors.bgBase,
        title: Text(l.twitchChatSettings),
      ),
      body: ListView(
        padding: const EdgeInsets.only(bottom: 24),
        children: [
          if (widget.skippedRules > 0)
            ListTile(
              leading: const Icon(Icons.warning_amber_rounded),
              title: Text(l.twitchChatSkippedRules(widget.skippedRules)),
            ),
          SwitchListTile(
            key: const ValueKey('chat-hide-bots'),
            title: Text(l.twitchChatHideBots),
            subtitle: Text(l.twitchChatHideBotsHint),
            value: _s.hideBots,
            onChanged: (v) => _set(_s.copyWith(hideBots: v)),
          ),
          SwitchListTile(
            key: const ValueKey('chat-highlight-own'),
            title: Text(l.twitchChatHighlightOwn),
            value: _s.highlightOwnName,
            onChanged: (v) => _set(_s.copyWith(highlightOwnName: v)),
          ),
          _StringList(
            key: const ValueKey('chat-keywords'),
            title: l.twitchChatKeywords,
            hint: l.twitchChatWordHint,
            items: _s.highlightKeywords,
            onChanged: (v) => _set(_s.copyWith(highlightKeywords: v)),
          ),
          _StringList(
            key: const ValueKey('chat-blocked'),
            title: l.twitchChatBlocked,
            hint: l.twitchChatLoginHint,
            footer: Text(l.twitchChatBlockedHint, style: caption),
            items: _s.blocked,
            normalise: chatLoginKey,
            onChanged: (v) => _set(_s.copyWith(blocked: v)),
          ),
          if (_s.hideBots)
            _StringList(
              key: const ValueKey('chat-extra-bots'),
              title: l.twitchChatExtraBots,
              hint: l.twitchChatLoginHint,
              items: _s.extraBots,
              normalise: chatLoginKey,
              onChanged: (v) => _set(_s.copyWith(extraBots: v)),
            ),
          _WordRules(
            rules: _s.words,
            check: widget.checkRule,
            onChanged: (v) => _set(_s.copyWith(words: v)),
          ),
        ],
      ),
    );
  }
}

class _Section extends StatelessWidget {
  const _Section(this.title);
  final String title;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(16, 20, 16, 4),
    child: Text(
      title,
      style: TextStyle(
        color: KColors.accentSoft,
        fontWeight: FontWeight.w600,
      ),
    ),
  );
}

/// A list of strings with an "add" field and remove buttons.
class _StringList extends StatefulWidget {
  const _StringList({
    super.key,
    required this.title,
    required this.hint,
    required this.items,
    required this.onChanged,
    this.normalise,
    this.footer,
  });

  final String title;
  final String hint;
  final List<String> items;
  final ValueChanged<List<String>> onChanged;
  final String Function(String)? normalise;
  final Widget? footer;

  @override
  State<_StringList> createState() => _StringListState();
}

class _StringListState extends State<_StringList> {
  final _field = TextEditingController();

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  void _add() {
    final raw = _field.text.trim();
    final v = widget.normalise?.call(raw) ?? raw;
    if (v.isEmpty) return;
    _field.clear();
    if (widget.items.contains(v)) return;
    widget.onChanged([...widget.items, v]);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Section(widget.title),
        if (widget.items.isEmpty)
          ListTile(
            dense: true,
            title: Text(
              l.twitchChatListEmpty,
              style: TextStyle(color: KColors.textTertiary),
            ),
          ),
        for (final item in widget.items)
          ListTile(
            dense: true,
            title: Text(item),
            trailing: IconButton(
              tooltip: l.twitchChatRemove(item),
              icon: const Icon(Icons.close_rounded),
              onPressed: () => widget.onChanged([
                ...widget.items.where((i) => i != item),
              ]),
            ),
          ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16),
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _field,
                  decoration: InputDecoration(hintText: widget.hint),
                  onSubmitted: (_) => _add(),
                ),
              ),
              TextButton(onPressed: _add, child: Text(l.twitchChatAdd)),
            ],
          ),
        ),
        if (widget.footer case final f?)
          Padding(padding: const EdgeInsets.fromLTRB(16, 8, 16, 0), child: f),
      ],
    );
  }
}

/// Word-filter rules: each with regex and hide/mask; the add field checks
/// the pattern before adding it.
class _WordRules extends StatefulWidget {
  const _WordRules({
    required this.rules,
    required this.onChanged,
    this.check,
  });

  final List<ChatWordRule> rules;
  final ValueChanged<List<ChatWordRule>> onChanged;
  final ChatRuleCheck? check;

  @override
  State<_WordRules> createState() => _WordRulesState();
}

class _WordRulesState extends State<_WordRules> {
  final _field = TextEditingController();
  var _regex = false;
  var _hide = false;
  String? _error;

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  void _add() {
    final p = _field.text.trim();
    if (p.isEmpty) return;
    final rule = ChatWordRule(p, regex: _regex, hide: _hide);
    final problem = widget.check?.call(rule);
    if (problem != null) {
      setState(() => _error = problem);
      return;
    }
    _field.clear();
    setState(() => _error = null);
    if (widget.rules.contains(rule)) return;
    widget.onChanged([...widget.rules, rule]);
  }

  void _replace(int i, ChatWordRule r) {
    final next = [...widget.rules];
    next[i] = r;
    widget.onChanged(next);
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final error = _error;
    return Column(
      key: const ValueKey('chat-words'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Section(l.twitchChatWords),
        if (widget.rules.isEmpty)
          ListTile(
            dense: true,
            title: Text(
              l.twitchChatListEmpty,
              style: TextStyle(color: KColors.textTertiary),
            ),
          ),
        for (final (i, r) in widget.rules.indexed)
          ListTile(
            dense: true,
            title: Text(
              r.pattern,
              style: r.regex ? const TextStyle(fontFamily: 'monospace') : null,
            ),
            subtitle: Text(
              [
                if (r.regex) l.twitchChatWordRegex,
                r.hide ? l.twitchChatWordHide : l.twitchChatWordMask,
              ].join(' · '),
            ),
            onTap: () => _replace(
              i,
              ChatWordRule(r.pattern, regex: r.regex, hide: !r.hide),
            ),
            trailing: IconButton(
              tooltip: l.twitchChatRemove(r.pattern),
              icon: const Icon(Icons.close_rounded),
              onPressed: () => widget.onChanged([
                for (final (j, x) in widget.rules.indexed)
                  if (j != i) x,
              ]),
            ),
          ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16),
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  key: const ValueKey('chat-word-field'),
                  controller: _field,
                  decoration: InputDecoration(
                    hintText: l.twitchChatWordHint,
                    errorText: error == null
                        ? null
                        : l.twitchChatInvalidRule(error),
                    errorMaxLines: 3,
                  ),
                  onChanged: (_) {
                    if (_error != null) setState(() => _error = null);
                  },
                  onSubmitted: (_) => _add(),
                ),
              ),
              TextButton(
                key: const ValueKey('chat-word-add'),
                onPressed: _add,
                child: Text(l.twitchChatAdd),
              ),
            ],
          ),
        ),
        CheckboxListTile(
          key: const ValueKey('chat-word-regex'),
          dense: true,
          title: Text(l.twitchChatWordRegex),
          value: _regex,
          onChanged: (v) => setState(() => _regex = v ?? false),
        ),
        CheckboxListTile(
          key: const ValueKey('chat-word-hide'),
          dense: true,
          title: Text(l.twitchChatWordHide),
          value: _hide,
          onChanged: (v) => setState(() => _hide = v ?? false),
        ),
      ],
    );
  }
}
