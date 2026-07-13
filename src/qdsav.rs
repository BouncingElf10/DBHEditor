//! Core logic for reading, editing and writing Detroit: Become Human `.qdsav`
//! save files (Quantic Dream engine).
//!
//! Container format: `[u32 LE uncompressed_size][zlib stream]`.
//! Payload: a serialized object graph of named variables. Relationship /
//! stability "gauges" store their current level as an adjective STRING value
//! (e.g. `$GUI_GAUGE_HANK_HOSTILE`) held by the variables
//! `_strAdjective<Char>` and `_strPastAdjective<Char>`.
//!
//! See `memory/qdsav-format.md` for the full reverse-engineered spec.

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read, Write};
use std::path::Path;

/// One relationship / stability gauge and its ordered levels (low -> high).
pub struct GaugeDef {
    /// Human-facing character/topic name shown in the UI.
    pub display: &'static str,
    /// Variable-name fragment: `_strAdjective<Char>` / `_strPastAdjective<Char>`.
    pub var_char: &'static str,
    /// Gauge label prefix: `$GUI_GAUGE_<PREFIX>_<LEVEL>`.
    pub prefix: &'static str,
    /// Ordered levels from lowest to highest relationship/stability.
    pub levels: &'static [&'static str],
    /// Short note shown in the UI to explain the gauge.
    pub note: &'static str,
}

/// All known gauges. Level orderings are low -> high.
pub const GAUGES: &[GaugeDef] = &[
    GaugeDef {
        display: "Connor — Software Stability",
        var_char: "Deviance",
        prefix: "DEVIANT",
        levels: &["MACHINE", "DEVIANT"],
        note: "MACHINE = stable/obedient, DEVIANT = unstable. Low stability (DEVIANT) unlocks deviant options, e.g. at Crossroads.",
    },
    GaugeDef {
        display: "Connor <-> Hank",
        var_char: "Hank",
        prefix: "HANK",
        levels: &["HOSTILE", "TENSE", "NEUTRAL", "WARM", "FRIEND"],
        note: "Higher = better relationship. Low relationship can lead Hank to quit / commit suicide.",
    },
    GaugeDef {
        display: "Connor <-> Amanda",
        var_char: "Amanda",
        prefix: "AMANDA",
        levels: &["BETRAYED", "DISTRUSTED", "NEUTRAL", "TRUSTED"],
        note: "Amanda's trust in Connor.",
    },
    GaugeDef {
        display: "Kara <-> Alice",
        var_char: "Alice",
        prefix: "ALICE",
        levels: &["DISTANT", "NEUTRAL", "WARM", "CLOSE", "TRUSTED", "FAMILY"],
        note: "Alice's bond with Kara.",
    },
    GaugeDef {
        display: "Kara <-> Luther",
        var_char: "Luther",
        prefix: "LUTHER",
        levels: &["WARY", "NEUTRAL", "TRUSTED", "CLOSE", "FAMILY"],
        note: "Luther's bond with Kara.",
    },
    GaugeDef {
        display: "Markus <-> North",
        var_char: "North",
        prefix: "NORTH",
        levels: &["HOSTILE", "RESENTFUL", "NEUTRAL", "FRIEND", "COMPANION", "LOVER"],
        note: "North's relationship with Markus.",
    },
    GaugeDef {
        display: "Markus <-> Josh",
        var_char: "Josh",
        prefix: "JOSH",
        levels: &["HOSTILE", "RESENTFUL", "NEUTRAL", "FRIEND", "COMPANION"],
        note: "Josh's relationship with Markus.",
    },
    GaugeDef {
        display: "Markus <-> Simon",
        var_char: "Simon",
        prefix: "SIMON",
        levels: &["HOSTILE", "RESENTFUL", "NEUTRAL", "FRIEND", "COMPANION"],
        note: "Simon's relationship with Markus.",
    },
    GaugeDef {
        display: "Markus — Leadership (Jericho)",
        var_char: "Leader",
        prefix: "LEADER",
        levels: &["REJECTED", "UNPOPULAR", "NEUTRAL", "RESPECTED", "ADMIRED", "LEADER"],
        note: "How Jericho's androids regard Markus as a leader.",
    },
    GaugeDef {
        display: "Markus — Public Opinion",
        var_char: "Public",
        prefix: "PUBLIC",
        levels: &["HATED", "HOSTILE", "SCEPTICAL", "NEUTRAL", "SYMPATHETIC", "SUPPORTIVE"],
        note: "How the human public views the android revolution.",
    },
];

/// Read and decompress a `.qdsav` file into its raw payload bytes.
pub fn read_qdsav(path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
    let raw = std::fs::read(path)?;
    if raw.len() < 6 {
        return Err(err("file too small to be a .qdsav"));
    }
    let declared = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    if raw[4] != 0x78 {
        return Err(err("missing zlib header (byte 4 != 0x78) — not a recognised .qdsav"));
    }
    let mut dec = ZlibDecoder::new(&raw[4..]);
    let mut payload = Vec::with_capacity(declared);
    dec.read_to_end(&mut payload)?;
    Ok(payload)
}

/// Compress `payload` and write it back in `.qdsav` container form.
pub fn write_qdsav(path: impl AsRef<Path>, payload: &[u8]) -> std::io::Result<()> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(9));
    enc.write_all(payload)?;
    let compressed = enc.finish()?;
    let mut out = Vec::with_capacity(compressed.len() + 4);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&compressed);
    std::fs::write(path, &out)
}

/// Serialization tag that immediately precedes a scalar variable's value.
const SCALAR_TAG: &[u8] = &[0x2b, 0x09, 0x22, 0x90];

/// A typed scalar value stored in a variable node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar {
    Bool(bool),
    Int(i32),
    Float(f32),
}

impl Scalar {
    fn type_nibble(self) -> u8 {
        match self {
            Scalar::Bool(_) => 0x2f,
            Scalar::Int(_) => 0x5f,
            Scalar::Float(_) => 0x7f,
        }
    }
}

/// An advanced scalar variable a user may want to edit directly.
pub struct AdvancedVar {
    pub var: &'static str,
    pub label: &'static str,
    pub help: &'static str,
}

/// Curated scalars that control Connor's deviancy — the numeric/flag backing
/// behind the display gauge. The relationship/stability ADJECTIVE (see GAUGES)
/// is only a display; scene branching reads these.
pub const ADVANCED_VARS: &[AdvancedVar] = &[
    AdvancedVar {
        var: "GM 0000XJaugeConnorDeviance",
        label: "Software instability — REAL value the game checks",
        help: "The flowchart gauge that dialogue conditions read. Negative = machine, positive = deviant. \
               Set to 100 to unlock Connor's deviant options. This is the one that matters.",
    },
    AdvancedVar {
        var: "GM bPlayerDeviant",
        label: "Connor has become deviant (flowchart)",
        help: "Flowchart-level 'player is deviant' flag. Try enabling alongside a high instability if needed.",
    },
    AdvancedVar {
        var: "TEMPbConnorDeviantUnlocked",
        label: "Unlock deviant option (scene-local flag)",
        help: "Scene-local temp flag; present only in some checkpoints. May be recomputed from the gauge on load.",
    },
    AdvancedVar {
        var: "bIsDeviant",
        label: "Connor is deviant (state flag)",
        help: "Marks Connor as having deviated. On its own this did not unlock the choice in testing.",
    },
    AdvancedVar {
        var: "_fPreviousDeviance",
        label: "Instability (display copy — not the gate)",
        help: "Mirrors the gauge for the on-screen meter. Editing this alone does NOT change branching.",
    },
];

/// Locate a scalar variable's value: `(value_offset, kind_nibble)`.
fn find_scalar(payload: &[u8], var_name: &str) -> Option<(usize, u8)> {
    let name = var_name.as_bytes();
    let mut from = 0;
    while let Some(rel) = find_sub(&payload[from..], name) {
        let name_end = from + rel + name.len();
        if let Some(tag) = find_sub(&payload[name_end..(name_end + 40).min(payload.len())], SCALAR_TAG) {
            let tag_pos = name_end + tag;
            let nibble = payload[tag_pos - 1];
            if matches!(nibble, 0x2f | 0x5f | 0x7f) {
                let vpos = tag_pos + SCALAR_TAG.len();
                let width = if nibble == 0x2f { 1 } else { 4 };
                if vpos + width <= payload.len() {
                    return Some((vpos, nibble));
                }
            }
        }
        from = name_end;
    }
    None
}

/// Read a scalar variable if present.
pub fn read_scalar(payload: &[u8], var_name: &str) -> Option<Scalar> {
    let (vpos, nibble) = find_scalar(payload, var_name)?;
    Some(match nibble {
        0x2f => Scalar::Bool(payload[vpos] != 0),
        0x5f => Scalar::Int(i32::from_le_bytes(payload[vpos..vpos + 4].try_into().ok()?)),
        0x7f => Scalar::Float(f32::from_le_bytes(payload[vpos..vpos + 4].try_into().ok()?)),
        _ => return None,
    })
}

/// Overwrite a scalar variable in place (fixed width — always offset-safe).
/// Returns Err if the variable is absent or the stored type differs.
pub fn set_scalar(payload: &mut [u8], var_name: &str, value: Scalar) -> std::io::Result<()> {
    let (vpos, nibble) = find_scalar(payload, var_name).ok_or_else(|| err("variable not present in this save"))?;
    if nibble != value.type_nibble() {
        return Err(err("stored type does not match the value being written"));
    }
    match value {
        Scalar::Bool(b) => payload[vpos] = b as u8,
        Scalar::Int(i) => payload[vpos..vpos + 4].copy_from_slice(&i.to_le_bytes()),
        Scalar::Float(f) => payload[vpos..vpos + 4].copy_from_slice(&f.to_le_bytes()),
    }
    Ok(())
}

/// The current reading of one gauge in a payload.
#[derive(Clone, Debug)]
pub struct GaugeReading {
    pub gauge_index: usize,
    /// Current level name (e.g. "HOSTILE"), or None if not present in this save.
    pub current: Option<String>,
}

fn err(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}

/// Find the byte range of the length-prefixed gauge value that belongs to the
/// variable `var_name`, searching a small window after the variable name.
/// Returns `(len_prefix_start, value_start, value_len)`.
fn find_value(payload: &[u8], var_name: &str, prefix: &str) -> Option<(usize, usize, usize)> {
    let name = var_name.as_bytes();
    let marker = format!("$GUI_GAUGE_{}_", prefix);
    let mut search_from = 0;
    while let Some(rel) = find_sub(&payload[search_from..], name) {
        let name_end = search_from + rel + name.len();
        // Look for the gauge value within a small window after the name.
        let window_end = (name_end + 96).min(payload.len());
        if let Some(vrel) = find_sub(&payload[name_end..window_end], marker.as_bytes()) {
            let value_start = name_end + vrel;
            if value_start >= 4 {
                let len_prefix_start = value_start - 4;
                let vlen = u32::from_le_bytes([
                    payload[len_prefix_start],
                    payload[len_prefix_start + 1],
                    payload[len_prefix_start + 2],
                    payload[len_prefix_start + 3],
                ]) as usize;
                // Sanity: length prefix must match a plausible value length.
                if vlen >= marker.len() && value_start + vlen <= payload.len() {
                    return Some((len_prefix_start, value_start, vlen));
                }
            }
        }
        search_from = name_end;
    }
    None
}

/// Read the current level for a gauge, if present.
fn read_level(payload: &[u8], g: &GaugeDef) -> Option<String> {
    let var = format!("_strAdjective{}", g.var_char);
    let (_, vstart, vlen) = find_value(payload, &var, g.prefix)?;
    let value = &payload[vstart..vstart + vlen];
    let level = std::str::from_utf8(value).ok()?;
    let suffix = level.strip_prefix(&format!("$GUI_GAUGE_{}_", g.prefix))?;
    Some(suffix.to_string())
}

/// Scan every known gauge in a payload.
pub fn scan(payload: &[u8]) -> Vec<GaugeReading> {
    GAUGES
        .iter()
        .enumerate()
        .map(|(i, g)| GaugeReading {
            gauge_index: i,
            current: read_level(payload, g),
        })
        .collect()
}

/// Set a gauge to `new_level` (must be one of the gauge's known levels).
/// Updates both the current and "past" adjective variables. Returns the number
/// of variables changed.
pub fn set_level(payload: &mut Vec<u8>, gauge_index: usize, new_level: &str) -> std::io::Result<usize> {
    let g = GAUGES.get(gauge_index).ok_or_else(|| err("invalid gauge index"))?;
    if !g.levels.contains(&new_level) {
        return Err(err("level not valid for this gauge"));
    }
    let new_value = format!("$GUI_GAUGE_{}_{}", g.prefix, new_level);
    let mut changed = 0;
    for var in [format!("_strAdjective{}", g.var_char), format!("_strPastAdjective{}", g.var_char)] {
        if replace_value(payload, &var, g.prefix, &new_value) {
            changed += 1;
        }
    }
    if changed == 0 {
        return Err(err("this gauge is not present in this save (character not encountered yet)"));
    }
    Ok(changed)
}

/// Replace the gauge value bound to `var_name`. Splices in the new value,
/// updating its 4-byte length prefix (handles differing lengths).
fn replace_value(payload: &mut Vec<u8>, var_name: &str, prefix: &str, new_value: &str) -> bool {
    let Some((len_prefix_start, value_start, old_len)) = find_value(payload, var_name, prefix) else {
        return false;
    };
    let mut replacement = Vec::with_capacity(4 + new_value.len());
    replacement.extend_from_slice(&(new_value.len() as u32).to_le_bytes());
    replacement.extend_from_slice(new_value.as_bytes());
    // Replace [len_prefix_start .. value_start+old_len) with the new prefix+value.
    payload.splice(len_prefix_start..value_start + old_len, replacement);
    true
}

/// Search helper: first index of `needle` in `hay`.
fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal payload chunk that mimics a string property:
    /// `[nameLen][name]<8 bytes metadata>[valueLen][value]`.
    fn prop(name: &str, value: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(name.len() as u32).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v.extend_from_slice(&[0xAA; 8]); // stand-in for ptr/hash metadata
        v.extend_from_slice(&(value.len() as u32).to_le_bytes());
        v.extend_from_slice(value.as_bytes());
        v
    }

    fn hank_gauge_index() -> usize {
        GAUGES.iter().position(|g| g.var_char == "Hank").unwrap()
    }

    #[test]
    fn scan_reads_current_level() {
        let mut payload = vec![0x11; 32];
        payload.extend(prop("_strAdjectiveHank", "$GUI_GAUGE_HANK_HOSTILE"));
        payload.extend(prop("_strPastAdjectiveHank", "$GUI_GAUGE_HANK_HOSTILE"));
        let readings = scan(&payload);
        let gi = hank_gauge_index();
        assert_eq!(readings[gi].current.as_deref(), Some("HOSTILE"));
    }

    #[test]
    fn set_level_changes_both_vars_and_reads_back() {
        let head = vec![0x01u8; 16];
        let tail = vec![0x02u8; 16];
        let mut payload = head.clone();
        payload.extend(prop("_strAdjectiveHank", "$GUI_GAUGE_HANK_HOSTILE"));
        payload.extend(prop("_strPastAdjectiveHank", "$GUI_GAUGE_HANK_HOSTILE"));
        payload.extend(&tail);

        let gi = hank_gauge_index();
        let changed = set_level(&mut payload, gi, "FRIEND").unwrap();
        assert_eq!(changed, 2, "both current and past adjectives updated");

        // Re-scan must report the new level.
        assert_eq!(scan(&payload)[gi].current.as_deref(), Some("FRIEND"));
        // Surrounding bytes preserved.
        assert_eq!(&payload[..16], &head[..]);
        assert_eq!(&payload[payload.len() - 16..], &tail[..]);
        // Value length prefix updated to the new (shorter) length.
        let vstart = find_sub(&payload, b"$GUI_GAUGE_HANK_FRIEND").unwrap();
        let len = u32::from_le_bytes(payload[vstart - 4..vstart].try_into().unwrap());
        assert_eq!(len as usize, "$GUI_GAUGE_HANK_FRIEND".len());
    }

    #[test]
    fn rejects_unknown_level() {
        let mut payload = prop("_strAdjectiveHank", "$GUI_GAUGE_HANK_HOSTILE");
        assert!(set_level(&mut payload, hank_gauge_index(), "BFFL").is_err());
    }

    #[test]
    fn absent_gauge_errors() {
        let mut payload = vec![0u8; 64]; // no gauge vars at all
        assert!(set_level(&mut payload, hank_gauge_index(), "FRIEND").is_err());
    }

    /// Build a scalar property: `[nameLen][name]<meta><TYPE>[2b 09 22 90][value]`.
    fn scalar_prop(name: &str, nibble: u8, value: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(name.len() as u32).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v.extend_from_slice(&[0xff, 0xff, 0xff, nibble]);
        v.extend_from_slice(SCALAR_TAG);
        v.extend_from_slice(value);
        v
    }

    #[test]
    fn scalar_read_and_write_fixed_width() {
        let mut payload = vec![0x00u8; 8];
        payload.extend(scalar_prop("TEMPbConnorDeviantUnlocked", 0x2f, &[0]));
        payload.extend(scalar_prop("_fPreviousDeviance", 0x7f, &(-5.0f32).to_le_bytes()));
        let tail = vec![0x99u8; 8];
        payload.extend(&tail);
        let before_len = payload.len();

        assert_eq!(read_scalar(&payload, "TEMPbConnorDeviantUnlocked"), Some(Scalar::Bool(false)));
        assert_eq!(read_scalar(&payload, "_fPreviousDeviance"), Some(Scalar::Float(-5.0)));

        set_scalar(&mut payload, "TEMPbConnorDeviantUnlocked", Scalar::Bool(true)).unwrap();
        set_scalar(&mut payload, "_fPreviousDeviance", Scalar::Float(100.0)).unwrap();

        assert_eq!(read_scalar(&payload, "TEMPbConnorDeviantUnlocked"), Some(Scalar::Bool(true)));
        assert_eq!(read_scalar(&payload, "_fPreviousDeviance"), Some(Scalar::Float(100.0)));
        assert_eq!(payload.len(), before_len, "scalar edits must not change length");
        assert_eq!(&payload[payload.len() - 8..], &tail[..]);
    }

    #[test]
    fn scalar_type_mismatch_errors() {
        let mut payload = scalar_prop("bIsDeviant", 0x2f, &[0]);
        assert!(set_scalar(&mut payload, "bIsDeviant", Scalar::Int(5)).is_err());
    }
}
