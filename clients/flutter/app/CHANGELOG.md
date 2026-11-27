# Changelog

## [0.1.0-alpha.23](https://github.com/DenisHumen/kinolune/compare/app-v0.1.0-alpha.22...app-v0.1.0-alpha.23) (2026-11-27)


### Новое · Features

* **lib:** live upload visible on all devices [LIB-028] ([#434](https://github.com/DenisHumen/kinolune/issues/434)) ([b8b0398](https://github.com/DenisHumen/kinolune/commit/b8b03985b4b3a0165b9a14e4f50d3736296b4057))
* **pkg:** Linux arm64 (aarch64) builds [PKG-026] ([#440](https://github.com/DenisHumen/kinolune/issues/440)) ([9052b1d](https://github.com/DenisHumen/kinolune/commit/9052b1d90e0a42c6c5429807fc1ecfbae39f534b))
* **play:** Blu-ray and disc images play by themselves [PLAY-017] ([#441](https://github.com/DenisHumen/kinolune/issues/441)) ([41a828a](https://github.com/DenisHumen/kinolune/commit/41a828ab1cc2736a5047af4e6b970ffa0ea25891))
* **play:** sleep timer and «Вы ещё смотрите?» [PLAY-022] ([#432](https://github.com/DenisHumen/kinolune/issues/432)) ([39688fd](https://github.com/DenisHumen/kinolune/commit/39688fd0cb9184d136efc349584e26c5a8ce41f0))
* **twi:** chat panel beside the Twitch embed player [TWI-041] ([#425](https://github.com/DenisHumen/kinolune/issues/425)) ([5057de0](https://github.com/DenisHumen/kinolune/commit/5057de0cd5eb7b89929762f46f56826185afedd3))
* **twi:** clip from the app [TWI-040] ([#442](https://github.com/DenisHumen/kinolune/issues/442)) ([57897d8](https://github.com/DenisHumen/kinolune/commit/57897d8d71fa23ca776852f783285a146e08bc46))
* **twi:** follow and unfollow from the app [TWI-033] ([#444](https://github.com/DenisHumen/kinolune/issues/444)) ([99a3f3b](https://github.com/DenisHumen/kinolune/commit/99a3f3b6fc14d3ab6c2693279181a2442c200726))
* **twi:** record live button and finished recording [TWI-035] ([#426](https://github.com/DenisHumen/kinolune/issues/426)) ([3a3a5a1](https://github.com/DenisHumen/kinolune/commit/3a3a5a1aa8d4eea997c72ebfe29e9c29402bf9cd))
* **twi:** sent-message bubble with playful animation [TWI-039] ([#424](https://github.com/DenisHumen/kinolune/issues/424)) ([4e86385](https://github.com/DenisHumen/kinolune/commit/4e86385afe7db446caf94d0204cbc37634726964))
* **twi:** Twitch archive as a series — streamer, game, parts [TWI-026] ([#446](https://github.com/DenisHumen/kinolune/issues/446)) ([7589d7e](https://github.com/DenisHumen/kinolune/commit/7589d7e3fc43cb9eb870d1c759b007ea9b33fb87))
* **twi:** Twitch browse tab, category streams and search [TWI-034] ([#420](https://github.com/DenisHumen/kinolune/issues/420)) ([b42e20b](https://github.com/DenisHumen/kinolune/commit/b42e20b8547cdc1bf3f90e386da42e7facebbc6d))
* **twi:** VOD chapters from GQL in the player [TWI-042] ([#462](https://github.com/DenisHumen/kinolune/issues/462)) ([b7976f0](https://github.com/DenisHumen/kinolune/commit/b7976f0989144f751ce7e42e41f77958b2c4f6d7))
* **ui:** ongoing shows — when the next episode airs [UI-057] ([#437](https://github.com/DenisHumen/kinolune/issues/437)) ([1711cc2](https://github.com/DenisHumen/kinolune/commit/1711cc21921df6390c210dd3c32217ad9af619ba))
* **ui:** season episodes in one horizontal row [UI-055] ([#419](https://github.com/DenisHumen/kinolune/issues/419)) ([7bd33c6](https://github.com/DenisHumen/kinolune/commit/7bd33c64f6f79574228cf4c21cc90780367f4158))


### Исправления · Bug Fixes

* **core,app:** fetch remote config after start [BUG-030] ([#429](https://github.com/DenisHumen/kinolune/issues/429)) ([4c34acc](https://github.com/DenisHumen/kinolune/commit/4c34acc424ee16254d1bb89975ac0220dcc93022))
* **play:** tests follow disc images being playable ([#451](https://github.com/DenisHumen/kinolune/issues/451)) ([6a29485](https://github.com/DenisHumen/kinolune/commit/6a29485c0a0cb56264a6c1036f1a6ae404324b0e))
* **repo:** gitleaks allows Twitch GQL query hashes in the remote config ([#454](https://github.com/DenisHumen/kinolune/issues/454)) ([9a62b73](https://github.com/DenisHumen/kinolune/commit/9a62b736d7b4f836217ec67d5d60358d78f16565))

## [0.1.0-alpha.22](https://github.com/DenisHumen/kinolune/compare/app-v0.1.0-alpha.21...app-v0.1.0-alpha.22) (2026-11-25)


### Новое · Features

* **meta:** review reason shows pure title similarity and adjustments separately [META-036] ([#389](https://github.com/DenisHumen/kinolune/issues/389)) ([47f4961](https://github.com/DenisHumen/kinolune/commit/47f4961fd689976f9fd789e149c9ec66270ade01))
* **twi:** Twitch account avatar and name in the app [TWI-032] ([#387](https://github.com/DenisHumen/kinolune/issues/387)) ([4d78dff](https://github.com/DenisHumen/kinolune/commit/4d78dffbc4f45c8f0872eb5bef2a22ee1b773f12))
* **ui:** total runtime and time left for series, season and movie [UI-056] ([#384](https://github.com/DenisHumen/kinolune/issues/384)) ([3c3fe8a](https://github.com/DenisHumen/kinolune/commit/3c3fe8a4fd85ff0e872f004604cd7ec124b8ad21))


### Исправления · Bug Fixes

* **play:** semantics crash on seek — Flutter stale geometry cache under hidden pages [BUG-015] ([#388](https://github.com/DenisHumen/kinolune/issues/388)) ([84887fd](https://github.com/DenisHumen/kinolune/commit/84887fdc523c6d5571242273fc3714cf7232d373))
* **twi:** Twitch plays on Linux Wayland instead of a black player [BUG-028] ([#383](https://github.com/DenisHumen/kinolune/issues/383)) ([7a324d5](https://github.com/DenisHumen/kinolune/commit/7a324d53529cf551f6fc09bf5318d83b9bff5e40))
* **ui:** diagnostic report shows where it is saved, full build version, Windows 11 and settings [BUG-024] ([#386](https://github.com/DenisHumen/kinolune/issues/386)) ([5911f62](https://github.com/DenisHumen/kinolune/commit/5911f6205af0229b50f86a9857da79f0a5f86f4c))
