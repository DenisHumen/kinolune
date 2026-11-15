// SPDX-License-Identifier: MPL-2.0
//! TMDB episode numbers of anime from what their files say (META-015).
//!
//! The scan files an episode the way its name is written: `Shingeki no
//! Kyojin - 38` is "absolute 38", `Shingeki no Kyojin Season 3 Part 2 - 05`
//! is "season 3, episode 5" (the season of the title). Neither is a TMDB
//! position: TMDB has the 38th episode as S03E01 and the fifth episode of
//! the second cour as S03E17. When the metadata run has matched an anime
//! work to a TMDB series, [`series`] lays the series out with the episode
//! numbering module (`pa_recognition::numbering`):
//!
//! - the TMDB season sizes from the details response;
//! - with the Anime-Lists list downloaded on the device (META-008,
//!   META-029), the AniDB entries of the series with their TMDB season and
//!   offset; entry sizes come from the next entry of the season or the
//!   season's end.
//!
//! [`renumber`] reads every file name of the work through that layout
//! (`Series::read_parsed`: the entry the title names, a running count, an
//! explicit season) and [`apply`] points the file's match at the episode
//! rows with the TMDB numbers (the running count kept as
//! `absolute_number`), so TMDB titles, stills and the AniDB order (META-029)
//! reach the right episodes. Explicit `SxxEyy` names stay as they are;
//! shows (non-anime works) are not renumbered.

use pa_anime_mappings::MappingIndex;
use pa_library::rusqlite::{OptionalExtension, Transaction, params};
use pa_metadata::tmdb::Details;
use pa_recognition::numbering::{Projections, Series};
use pa_recognition::parse::parse;
use pa_recognition::title::TitleKey;

use crate::Error;

/// SQL condition (on `matches AS m`) of a match a newer scorer or the
/// renumbering may decide again: automatic and not confirmed (`auto`,
/// `auto_review`; never `user_confirmed` or `user_set`), and not under a
/// user folder rule of its source (`folder_rules`, META-017).
macro_rules! recomputable {
    () => {
        "(m.state IN ('auto', 'auto_review') AND NOT EXISTS (
            SELECT 1 FROM files rf JOIN folder_rules fr ON fr.source_id = rf.source_id
            WHERE rf.id = m.file_id
              AND (trim(fr.path_prefix, '/') = ''
                   OR ltrim(rf.path, '/') = trim(fr.path_prefix, '/')
                   OR substr(ltrim(rf.path, '/'), 1, length(trim(fr.path_prefix, '/')) + 1)
                      = trim(fr.path_prefix, '/') || '/')))"
    };
}
pub(crate) use recomputable;

/// The episodes of one file of a work in every order the series has data
/// for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renumbered {
    /// `files.id`.
    pub file_id: i64,
    /// One per episode of the file, in file order; each has a TMDB position.
    pub episodes: Vec<Projections>,
}

/// The layout of TMDB TV series `tmdb`: season sizes `(season, regular
/// episodes)` and, from `mappings`, its AniDB entries.
#[must_use]
pub fn series(
    tmdb: u32,
    seasons: impl IntoIterator<Item = (u32, u32)>,
    mappings: Option<&MappingIndex>,
) -> Series {
    let mut s = mappings
        .and_then(|m| Series::from_mappings(m, Some(tmdb), None))
        .unwrap_or_default();
    s.tmdb = Some(tmdb);
    s.tmdb_seasons = seasons
        .into_iter()
        .filter(|&(season, count)| season >= 1 && count > 0)
        .collect();
    s.infer_counts();
    s
}

/// [`series`] of a TMDB TV details response.
#[must_use]
pub fn series_of_details(details: &Details, mappings: Option<&MappingIndex>) -> Option<Series> {
    let tmdb = u32::try_from(details.id).ok()?;
    Some(series(
        tmdb,
        details
            .seasons
            .iter()
            .map(|s| (s.season_number, s.episode_count)),
        mappings,
    ))
}

/// TMDB positions of the files `(file id, relative path)` of an anime work
/// known by `titles` (local titles first). A file is left out when any of
/// its episodes has no TMDB position (no layout data, a `.5` episode, an
/// extra).
#[must_use]
pub fn renumber(series: &Series, files: &[(i64, String)], titles: &[String]) -> Vec<Renumbered> {
    let hint = titles.iter().find_map(|t| series.entry_named(t));
    files
        .iter()
        .filter_map(|(id, path)| {
            let parsed = parse(path).best().parsed.clone();
            let key = parsed.title.as_deref().map(TitleKey::new);
            let own = parsed.title.as_deref().and_then(|t| series.entry_named(t));
            // A title with its own season or part (`Haikyu!! Season 2`) is
            // not the entry another title of the work names (`Haikyu!!`).
            let qualified = key.is_some_and(|k| k.season.is_some() || k.part.is_some());
            let episodes = series.read_parsed(&parsed, own.or(hint.filter(|_| !qualified)));
            let placed = !episodes.is_empty()
                && episodes
                    .iter()
                    .all(|p| p.tmdb.is_some_and(|t| t.season >= 1 && t.episode >= 1));
            placed.then_some(Renumbered {
                file_id: *id,
                episodes,
            })
        })
        .collect()
}

/// TMDB seasons the renumbered episodes fall into.
#[must_use]
pub fn seasons(renumbered: &[Renumbered]) -> Vec<u32> {
    let mut out: Vec<u32> = renumbered
        .iter()
        .flat_map(|r| r.episodes.iter().filter_map(|p| p.tmdb.map(|t| t.season)))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The regular episode row of `work` at TMDB `season`x`episode`, created if
/// needed; its running count is filled when unknown.
fn episode_row(
    tx: &Transaction<'_>,
    work: i64,
    season: i64,
    episode: i64,
    absolute: Option<i64>,
) -> Result<i64, Error> {
    let found: Option<i64> = tx
        .query_row(
            "SELECT id FROM episodes
             WHERE work_id = ?1 AND season_number = ?2 AND episode_number = ?3
               AND episode_type = 'regular'
             ORDER BY id LIMIT 1",
            params![work, season, episode],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = found {
        if absolute.is_some() {
            tx.execute(
                "UPDATE episodes SET absolute_number = ?2 WHERE id = ?1 AND absolute_number IS NULL",
                params![id, absolute],
            )?;
        }
        return Ok(id);
    }
    tx.execute(
        "INSERT INTO episodes (work_id, episode_type, season_number, episode_number,
                               absolute_number)
         VALUES (?1, 'regular', ?2, ?3, ?4)",
        params![work, season, episode, absolute],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Points the automatic, unconfirmed matches of the renumbered files of
/// `work` (never user decisions or files under a user folder rule) at the
/// episode rows with their TMDB numbers; rows no match uses any more are
/// removed.
/// Returns the number of episodes moved.
pub(crate) fn apply(
    tx: &Transaction<'_>,
    work: i64,
    renumbered: &[Renumbered],
) -> Result<usize, Error> {
    let mut moved = 0;
    let mut left: Vec<i64> = Vec::new();
    for r in renumbered {
        let Some(m): Option<i64> = tx
            .query_row(
                concat!(
                    "SELECT m.id FROM matches m WHERE m.file_id = ?1 AND m.work_id = ?2 AND ",
                    recomputable!(),
                    " ORDER BY m.id DESC LIMIT 1"
                ),
                params![r.file_id, work],
                |row| row.get(0),
            )
            .optional()?
        else {
            continue;
        };
        let current: Vec<(i64, i64, String)> = {
            let mut stmt = tx.prepare_cached(
                "SELECT me.position, me.episode_id, e.episode_type
                 FROM match_episodes me JOIN episodes e ON e.id = me.episode_id
                 WHERE me.match_id = ?1 ORDER BY me.position",
            )?;
            stmt.query_map([m], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?
        };
        if current.len() != r.episodes.len() || current.iter().any(|(_, _, t)| t != "regular") {
            continue;
        }
        for ((position, old), p) in current.iter().map(|(a, b, _)| (a, b)).zip(&r.episodes) {
            let Some(t) = p.tmdb else { continue };
            let new = episode_row(
                tx,
                work,
                i64::from(t.season),
                i64::from(t.episode),
                p.tvdb_absolute.map(i64::from),
            )?;
            if new == *old {
                continue;
            }
            let n = tx.execute(
                "UPDATE OR IGNORE match_episodes SET episode_id = ?3
                 WHERE match_id = ?1 AND position = ?2",
                params![m, position, new],
            )?;
            if n > 0 {
                moved += 1;
                left.push(*old);
            }
        }
    }
    for old in left {
        tx.execute(
            "DELETE FROM episodes WHERE id = ?1
               AND NOT EXISTS (SELECT 1 FROM match_episodes WHERE episode_id = ?1)",
            [old],
        )?;
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pa_recognition::numbering::SeasonEpisode;

    const LIST: &str = r#"<anime-list>
  <anime anidbid="9541" tvdbid="267440" defaulttvdbseason="1" tmdbtv="1429" tmdbseason="1"><name>Shingeki no Kyojin</name></anime>
  <anime anidbid="10944" tvdbid="267440" defaulttvdbseason="2" tmdbtv="1429" tmdbseason="2"><name>Shingeki no Kyojin Season 2</name></anime>
  <anime anidbid="13241" tvdbid="267440" defaulttvdbseason="3" tmdbtv="1429" tmdbseason="3"><name>Shingeki no Kyojin Season 3</name></anime>
  <anime anidbid="14444" tvdbid="267440" defaulttvdbseason="3" episodeoffset="12" tmdbtv="1429" tmdbseason="3" tmdboffset="12"><name>Shingeki no Kyojin Season 3 Part 2</name></anime>
</anime-list>"#;

    fn tmdb(r: &[Renumbered]) -> Vec<SeasonEpisode> {
        r.iter()
            .flat_map(|x| x.episodes.iter().filter_map(|p| p.tmdb))
            .collect()
    }

    fn files(paths: &[&str]) -> Vec<(i64, String)> {
        paths
            .iter()
            .enumerate()
            .map(|(i, p)| (i64::try_from(i).unwrap(), (*p).to_owned()))
            .collect()
    }

    #[test]
    fn running_count_lands_in_tmdb_seasons_without_a_list() {
        let s = series(1429, [(0, 8), (1, 25), (2, 12), (3, 22)], None);
        let r = renumber(
            &s,
            &files(&[
                "Anime/Attack on Titan/[SubsPlease] Attack on Titan - 38 (1080p).mkv",
                "Anime/Attack on Titan/[SubsPlease] Attack on Titan - 59 (1080p).mkv",
                "Anime/Attack on Titan/[SubsPlease] Attack on Titan - 60 (1080p).mkv",
                "TV/Attack on Titan/Season 03/Attack.on.Titan.S03E15.1080p.WEB.mkv",
            ]),
            &["Attack on Titan".to_owned()],
        );
        assert_eq!(
            tmdb(&r),
            [
                SeasonEpisode::new(3, 1),
                SeasonEpisode::new(3, 22),
                SeasonEpisode::new(3, 15)
            ]
        );
        assert_eq!(r[0].episodes[0].tvdb_absolute, Some(38));
        assert_eq!(seasons(&r), [3]);
    }

    #[test]
    fn entry_titles_need_the_list() {
        let mut index = MappingIndex::new();
        index.add_anime_lists(pa_anime_mappings::anime_lists::parse(LIST).unwrap());
        let s = series(1429, [(1, 25), (2, 12), (3, 22)], Some(&index));
        assert_eq!(
            s.entries.iter().map(|e| e.episodes).collect::<Vec<_>>(),
            [Some(25), Some(12), Some(12), Some(10)]
        );
        let names = files(&[
            "Anime/Shingeki no Kyojin Season 3 Part 2/[Grp] Shingeki no Kyojin Season 3 Part 2 - 05 (1080p).mkv",
            "Anime/Shingeki no Kyojin Season 3/[Grp] Shingeki no Kyojin Season 3 - 15 (1080p).mkv",
            "Anime/Shingeki no Kyojin/[Grp] Shingeki no Kyojin - 38 (1080p).mkv",
            "Anime/Shingeki no Kyojin/[Grp] Shingeki no Kyojin - NCOP1 (1080p).mkv",
        ]);
        let r = renumber(&s, &names, &[]);
        assert_eq!(
            tmdb(&r),
            [
                SeasonEpisode::new(3, 17),
                SeasonEpisode::new(3, 15),
                SeasonEpisode::new(3, 1)
            ]
        );
        // Without the list the second cour's own numbering cannot be placed
        // and stays as written.
        let bare = series(1429, [(1, 25), (2, 12), (3, 22)], None);
        let r = renumber(&bare, &names[..1], &[]);
        assert_eq!(tmdb(&r), [SeasonEpisode::new(3, 5)]);
    }
}
