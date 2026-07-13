# DBHEditor

A save editor for **Detroit: Become Human** (PC). Edit the relationship and
stability values in a checkpoint before you load it.

- **Unlock Connor's deviant path** — one click raises his software instability so
  the "become deviant" dialogue choice appears (e.g. confronting North).
- Set any character relationship (Hank, North, Amanda, Alice, Luther, Josh,
  Simon, Markus' leadership, public opinion) to any level.
- Automatic per-save backups, plus **full-folder backup & restore**.

## Run it

```
cargo run --release --bin DBHEditor
```

The window opens and automatically finds your saves at
`%USERPROFILE%\Saved Games\Quantic Dream\Detroit Become Human`. If it can't, use
**Open folder…**.

## Using it

1. (Recommended) Click **💾 Full backup** once — it copies your whole save folder
   aside so you can always roll back.
2. Pick the checkpoint you're about to load from the left (grouped by chapter,
   filterable). Edited saves show a ✎ mark.
3. Adjust things:
   - **Connor's deviant path** card → *Unlock deviant option*.
   - **Relationships & stability** → click a level per character.
   - **Advanced** → fine-tune the raw deviancy variables if you want.
4. Click **💾 Save changes**, then load that checkpoint in-game.

### Undo

- **↺ Undo last save** restores the selected checkpoint from its most recent
  automatic backup.
- **↩ Restore backup…** rolls the entire save folder back to a full backup (your
  current state is snapshotted first, just in case).

## How it works (safety)

Saves are Quantic Dream `.qdsav` files: a 4-byte size header + a zlib stream
holding a graph of named variables. Scene branching is driven by *numeric
flowchart variables* (e.g. `GM 0000XJaugeConnorDeviance`, Connor's instability) —
the on-screen adjective gauges are only a display. The editor changes the values
the game actually reads. Every write makes a backup first, so nothing is
irreversible. Don't edit while the game is running.

## Dev

- Tests: `cargo test`
- Inspect a raw save: `cargo run --bin analyze -- "<file.qdsav>" [out.bin]`
- Format notes live in the project memory file `qdsav-format.md`.
