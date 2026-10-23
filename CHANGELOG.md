# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

## [0.1.0] - 2026-10-07

Initial release.

### Added

- `parse_ofx` / `parse_ofx_with_scale` — OFX 1.x SGML statements to
  `recon_kit::Statement`, with signed amounts normalised into
  `Direction` + unsigned magnitude at the boundary.
- Float-free decimal parsing: `"0.07"` is exactly 7 minor units, because the
  parser never constructs a float on the path from string to minor units.
- `YYYYMMDD[HHMMSS]` date normalisation, refusing non-numeric input.
- Missing `FITID`, missing amounts, malformed dates, unknown currency, and
  absent closing balance are errors, not generated defaults.
