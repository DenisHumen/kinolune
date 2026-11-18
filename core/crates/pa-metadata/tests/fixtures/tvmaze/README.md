<!-- SPDX-License-Identifier: MPL-2.0 -->
# Кассеты TVmaze, TheTVDB и Fanart.tv для тестов (META-012)

`tvmaze/cassette.jsonl`, `tvdb/cassette.jsonl`, `fanart/cassette.jsonl` и `opensubtitles/identify.jsonl` — HTTP-кассеты для `ReplayTransport` (формат — в rustdoc `pa_metadata::replay`).

**Данные синтетические.** Структура повторяет ответы TVmaze (`/search/shows`, `/lookup/shows`, `/shows/{id}/episodesbydate`), TheTVDB v4 (`POST /login`, `/search`, `/series/{id}/extended`, `/series/{id}/episodes/default?airDate=`), Fanart.tv v3 (`/movies/{tmdb}`, `/tv/{tvdb}`) и поиска OpenSubtitles по moviehash (`feature_details` с `tmdb_id` и `parent_tmdb_id`), но шоу («The Late Desk», «Paper Lanterns»), ID (`9xxxx`, `39xxxx`, `19xxxx`), тексты и URL картинок выдуманы. Сетевых вызовов при записи не было; ключ в кассете TheTVDB — `test-key`, токен — `synthetic-token`.

Сценарии: daily-шоу по дате эфира (есть эпизод, 404 — нет эпизода), поиск по названию, lookup по TheTVDB и IMDb id, `429` → `200`; вход TheTVDB, неверный ключ, повторный вход после `401`, ID TMDB из `remoteIds`; выбор логотипа и clear art по языку и лайкам, `404`; голосование субтитров с `moviehash_match` за фильм или эпизод.

Сверка формы с живыми API (локально, без коммита записи): `RecordingTransport` поверх `UreqTransport`; ключи TheTVDB и Fanart.tv — только свои, в `.env.local`.
