# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Detroit: Become Human (PC) **save editor**. It edits the relationship and
stability "gauges" stored in a checkpoint save so the player can change how a
scene branches before loading it (e.g. drop Connor's stability to DEVIANT to
unlock a deviant option at Crossroads, or raise Connor↔Hank so Hank doesn't
quit / commit suicide). GUI built with `eframe`/`egui`.

## Commands

- Run the GUI: `cargo run --bin DBHEditor`
- Build (release): `cargo build --release`
- Tests: `cargo test` (unit tests in `src/qdsav.rs` and `src/saves.rs`); single
  test: `cargo test set_level_changes_both_vars_and_reads_back`
- Inspect a raw save (dev tool): `cargo run --bin analyze -- "<file.qdsav>" [out.bin]`
  — decompresses and hexdumps a `.qdsav`.
- Lint/format: `cargo clippy`, `cargo fmt`

## Architecture

The crate is a library (`src/lib.rs`, lib name `dbheditor`) plus binaries.

- `src/qdsav.rs` — the format core. Container read/write, the `GAUGES` model,
  `scan`/`set_level` for adjective gauges, and `read_scalar`/`set_scalar` +
  `ADVANCED_VARS` for the numeric/flag variables that actually gate branching.
  **All save-format knowledge lives here.**
- `src/saves.rs` — locate the DBH save folder, enumerate checkpoints (files
  under a `Checkpoints` dir), per-file `.bak-<epoch>` backups + undo, and
  **full-folder backup/restore** (`create_full_backup` / `list_full_backups` /
  `restore_full_backup`). Full backups go in a sibling `DBHEditor Backups`
  folder so they are never scanned as saves.
- `src/main.rs` — the egui GUI: chapter-grouped checkpoint list, a "Connor's
  deviant path" quick-action card, per-character level chips, an Advanced
  scalars section, and the backup/restore + confirm windows.
- `src/bin/analyze.rs` — dev-only raw-format inspector.

## Save format (the critical, non-obvious part)

Fully documented in the memory file `qdsav-format.md`. Essentials:

- **Container:** `[u32 LE uncompressed_size][zlib stream]` (zlib magic `78 9c`).
  Decompress from byte 4; on write, recompress (any level) and re-prefix the
  size. Byte-identical recompression is not required.
- **Payload:** a serialized object graph of length-prefixed named variables
  (`_str*`, `_b*`, …), each carrying a GUID and type hashes.
- **Adjective gauges are DISPLAY ONLY.** A gauge's level is stored as a string
  (e.g. `$GUI_GAUGE_HANK_HOSTILE`) in `_strAdjective<Char>`/`_strPastAdjective<Char>`;
  editing it moves the on-screen meter but does NOT change branching.
- **Branching reads numeric flowchart variables** named `GM <name>` (fixed-width
  scalars, encoded `... ff ff ff <TYPE> [2b 09 22 90] <VALUE>`; TYPE `2f`=bool,
  `5f`=int32, `7f`=float32). Confirmed in-game: setting
  `GM 0000XJaugeConnorDeviance` (Connor's instability) to 100 unlocks his deviant
  choice. Scalar edits are fixed-width → always offset-safe.
- **Internal references are ABSOLUTE FILE OFFSETS**, so string-length changes
  shift data. Same-length level swaps are always safe; cross-length swaps splice
  + fix the length prefix (works because the loader re-links by GUID). Scalar
  edits never change length. Editing always writes a `.bak-<epoch>` backup first.
