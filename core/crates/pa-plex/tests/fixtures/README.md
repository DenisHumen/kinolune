<!-- SPDX-License-Identifier: MPL-2.0 -->

# Ответы plex.tv и Plex Media Server для тестов (NET-012)

Синтетические ответы в форме, которую описывает публичная документация Plex API
(`Accept: application/json`): ни одного реального аккаунта, токена, адреса или
сервера. `tests/plex.rs` отдаёт их со своего локального HTTP-сервера; `{HOST}`
заменяется на его адрес (`127.0.0.1:<порт>`).

| Файл | Запрос |
|---|---|
| `pin_new.json`, `pin_linked.json` | `POST /api/v2/pins`, `GET /api/v2/pins/{id}` до и после ввода кода на plex.tv/link |
| `user.json` | `GET /api/v2/user` |
| `home_users.json` | `GET /api/v2/home/users` |
| `resources.json` | `GET /api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1` |
| `identity.json` | `GET /identity` |
| `sections.json` | `GET /library/sections` |
| `movies.json` | `GET /library/sections/1/all?type=1&includeGuids=1` (по одному элементу на страницу) |
| `shows.json`, `episodes.json` | `GET /library/sections/2/all?type=2`, `type=4` |
| `tracks.json` | `GET /library/sections/3/all?type=10`, `GET /playlists/30/items` |
| `playlists.json` | `GET /playlists?playlistType=audio` |
| `video_playlists.json`, `video_playlist_items.json` | `GET /playlists?playlistType=video`, `GET /playlists/40/items` (NET-013: серия, песня и фильм — песня в видеоплейлист не попадает) |

NET-013: стенд хранит состояние пользователя по элементам (`viewOffset`, `viewCount`, `lastViewedAt`) поверх
фикстур, меняет его по `/:/timeline`, `/:/scrobble`, `/:/unscrobble`, `/:/progress` и
`PUT /actions/removeFromContinueWatching` так, как это делает сервер (timeline `stopped` дальше 90 % — «просмотрено»),
и отдаёт в листингах и `/library/metadata/{id}`; `Mock::set_user_state` изображает изменение в Plex Web.
