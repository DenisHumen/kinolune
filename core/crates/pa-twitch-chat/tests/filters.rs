// SPDX-License-Identifier: MPL-2.0
//! Viewer filters (TWI-024): block list, word filter (plain and regex,
//! hide and mask), mentions and "hide bots". All names are made up.

use pa_twitch_chat::{
    ChatFilters, ChatMessage, ChatUser, EmoteRange, FilterSettings, HideReason, ReplyInfo,
    WordAction, WordRule,
};

fn msg(login: &str, text: &str) -> ChatMessage {
    ChatMessage {
        id: format!("id-{login}"),
        user: ChatUser {
            id: format!("9{}", login.len()),
            login: login.to_owned(),
            display_name: login.to_owned(),
            ..ChatUser::default()
        },
        text: text.to_owned(),
        ..ChatMessage::default()
    }
}

fn rule(pattern: &str, regex: bool, action: WordAction) -> WordRule {
    WordRule {
        pattern: pattern.to_owned(),
        regex,
        action,
    }
}

fn filters(settings: FilterSettings) -> ChatFilters {
    let (f, errors) = ChatFilters::new(&settings);
    assert!(errors.is_empty(), "{errors:?}");
    f
}

#[test]
fn empty_settings_pass_everything() {
    let f = filters(FilterSettings::default());
    assert!(f.is_empty());
    let mut m = msg("plum_viewer", "hello there");
    let v = f.apply(&mut m);
    assert_eq!(v.hidden, None);
    assert!(!v.mention && !v.masked);
    assert_eq!(m.text, "hello there");
}

#[test]
fn block_list_matches_login_any_case_and_user_id() {
    let f = filters(FilterSettings {
        blocked: vec!["@Noisy_Otter".into(), " 93 ".into()],
        ..FilterSettings::default()
    });
    let mut m = msg("noisy_otter", "spam spam");
    assert_eq!(f.apply(&mut m).hidden, Some(HideReason::Blocked));
    // User id "93": login of three characters in `msg`.
    let mut m = msg("abc", "hi");
    assert_eq!(f.apply(&mut m).hidden, Some(HideReason::Blocked));
    let mut m = msg("quiet_otter", "hi");
    assert_eq!(f.apply(&mut m).hidden, None);
    assert!(f.is_blocked("", "NOISY_OTTER"));
}

#[test]
fn hide_bots_uses_the_known_list_and_the_users_list() {
    let settings = FilterSettings {
        hide_bots: true,
        extra_bots: vec!["Helper_Robo_9000".into()],
        ..FilterSettings::default()
    };
    let f = filters(settings.clone());
    let mut m = msg("nightbot", "!commands");
    assert_eq!(f.apply(&mut m).hidden, Some(HideReason::Bot));
    let mut m = msg("helper_robo_9000", "Welcome!");
    assert_eq!(f.apply(&mut m).hidden, Some(HideReason::Bot));
    let mut m = msg("robo_fan", "I like robots");
    assert_eq!(f.apply(&mut m).hidden, None);

    // Off: bots are shown.
    let f = filters(FilterSettings {
        hide_bots: false,
        ..settings
    });
    let mut m = msg("nightbot", "!commands");
    assert_eq!(f.apply(&mut m).hidden, None);
}

#[test]
fn plain_word_masks_whole_words_only_and_keeps_length() {
    let f = filters(FilterSettings {
        words: vec![rule("spoiler", false, WordAction::Mask)],
        ..FilterSettings::default()
    });
    let mut m = msg("plum_viewer", "SPOILER: the end. spoilers and nospoiler stay");
    let v = f.apply(&mut m);
    assert!(v.masked);
    assert_eq!(v.hidden, None);
    assert_eq!(m.text, "*******: the end. spoilers and nospoiler stay");
}

#[test]
fn masking_counts_characters_and_drops_touched_emotes() {
    let f = filters(FilterSettings {
        words: vec![rule("плохо", false, WordAction::Mask)],
        ..FilterSettings::default()
    });
    // "Ура плохо FakeEmote": emote at chars 10..19 survives; the one
    // over the masked word goes.
    let mut m = msg("plum_viewer", "Ура плохо FakeEmote");
    m.emotes = vec![
        EmoteRange {
            id: "e1".into(),
            start: 4,
            end: 9,
        },
        EmoteRange {
            id: "e2".into(),
            start: 10,
            end: 19,
        },
    ];
    assert!(f.apply(&mut m).masked);
    assert_eq!(m.text, "Ура ***** FakeEmote");
    assert_eq!(m.text.chars().count(), 19);
    assert_eq!(m.emotes.len(), 1);
    assert_eq!(m.emotes[0].id, "e2");
}

#[test]
fn regex_rules_hide_and_mask() {
    let f = filters(FilterSettings {
        words: vec![
            rule(r"buy\s+followers", true, WordAction::Hide),
            rule(r"\d{3}-\d{4}", true, WordAction::Mask),
        ],
        ..FilterSettings::default()
    });
    let mut m = msg("sly_seller", "Cheap! BUY   followers at example.invalid");
    assert_eq!(f.apply(&mut m).hidden, Some(HideReason::Word));
    let mut m = msg("plum_viewer", "call 555-0100 or 555-0199");
    assert!(f.apply(&mut m).masked);
    assert_eq!(m.text, "call ******** or ********");
}

#[test]
fn bad_rules_are_reported_and_skipped() {
    let (f, errors) = ChatFilters::new(&FilterSettings {
        words: vec![
            rule("(open", true, WordAction::Hide),
            rule("ok", false, WordAction::Hide),
            rule("a*", true, WordAction::Hide),
        ],
        ..FilterSettings::default()
    });
    assert_eq!(
        errors.iter().map(|e| e.index).collect::<Vec<_>>(),
        vec![0, 2]
    );
    let mut m = msg("plum_viewer", "ok then");
    assert_eq!(f.apply(&mut m).hidden, Some(HideReason::Word));
    // The empty-matching rule did not turn into "hide everything".
    let mut m = msg("plum_viewer", "fine");
    assert_eq!(f.apply(&mut m).hidden, None);
}

#[test]
fn a_pathological_pattern_does_not_hang() {
    // Catastrophic for a backtracking engine; linear here.
    let f = filters(FilterSettings {
        words: vec![rule(r"(a+)+$", true, WordAction::Hide)],
        ..FilterSettings::default()
    });
    let mut m = msg("plum_viewer", &format!("{}!", "a".repeat(5000)));
    let start = std::time::Instant::now();
    let v = f.apply(&mut m);
    assert_eq!(v.hidden, None);
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn mentions_of_own_name_keywords_and_replies() {
    let f = filters(FilterSettings {
        own_login: Some("Moon_Watcher".into()),
        highlight_keywords: vec!["giveaway".into()],
        ..FilterSettings::default()
    });
    let mut m = msg("plum_viewer", "hey @moon_watcher look");
    assert!(f.apply(&mut m).mention);
    let mut m = msg("plum_viewer", "MOON_WATCHER!");
    assert!(f.apply(&mut m).mention);
    let mut m = msg("plum_viewer", "moon_watchers unite");
    assert!(!f.apply(&mut m).mention, "whole word only");
    let mut m = msg("plum_viewer", "Giveaway at the end?");
    assert!(f.apply(&mut m).mention);
    let mut m = msg("plum_viewer", "nice play");
    m.reply = Some(ReplyInfo {
        parent_user_login: "moon_watcher".into(),
        ..ReplyInfo::default()
    });
    assert!(f.apply(&mut m).mention);
    // Own messages are not mentions and are never blocked.
    let mut m = msg("moon_watcher", "moon_watcher here, giveaway soon");
    assert!(!f.apply(&mut m).mention);
}

#[test]
fn own_messages_are_not_blocked_or_treated_as_bots() {
    let f = filters(FilterSettings {
        own_login: Some("moon_watcher".into()),
        blocked: vec!["moon_watcher".into()],
        hide_bots: true,
        extra_bots: vec!["moon_watcher".into()],
        ..FilterSettings::default()
    });
    let mut m = msg("moon_watcher", "hi");
    assert_eq!(f.apply(&mut m).hidden, None);
}
