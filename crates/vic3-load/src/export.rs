//! Surgical plaintext patches of Vic3 `.v3` gamestate text.
//!
//! This module never round-trips the serde IR (lossy). It locates
//! `building_manager.database` entries in the original uncompressed text and
//! rewrites `production_methods` / `levels` in place.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// JSON delta applied to a plaintext save.
///
/// `extra_levels` is additive on the saved `levels` / `level` field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SavePatch {
    #[serde(default)]
    pub production_methods: Vec<ProductionMethodPatch>,
    #[serde(default)]
    pub extra_levels: Vec<ExtraLevelsPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProductionMethodPatch {
    pub building_id: u32,
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ExtraLevelsPatch {
    pub building_id: u32,
    pub extra_levels: u32,
}

impl SavePatch {
    fn is_empty(&self) -> bool {
        self.production_methods.is_empty() && self.extra_levels.is_empty()
    }
}

/// Failure while patching a plaintext save.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// Ironman / binary envelope. Text patching cannot rewrite token streams.
    #[error("ironman/binary saves are not supported")]
    BinaryNotSupported,
    /// Conservative: never invent a building that is not in the save.
    #[error("building id {0} not found in building_manager.database")]
    BuildingNotFound(u32),
    #[error("building_manager.database not found in save")]
    MissingBuildingManager,
    #[error("building {building_id} has no {field} field to patch")]
    MissingField {
        building_id: u32,
        field: &'static str,
    },
    #[error("zip save has no gamestate member")]
    MissingGamestate,
    #[error("failed to read zip save: {0}")]
    Zip(String),
    #[error("malformed Clausewitz braces in save")]
    Malformed,
}

/// Patch `original` and return new bytes. The input slice is never written.
///
/// Zip input is re-emitted as a single `gamestate` member. Raw text input
/// stays raw text.
pub fn export_save(original: &[u8], patch: &SavePatch) -> Result<Vec<u8>, ExportError> {
    if patch.is_empty() {
        return Ok(original.to_vec());
    }
    if looks_like_zip(original) {
        let gamestate = read_zip_gamestate(original)?;
        reject_binary(&gamestate)?;
        let patched = patch_gamestate(&gamestate, patch)?;
        return write_zip_gamestate(&patched);
    }
    reject_binary(original)?;
    patch_gamestate(original, patch)
}

fn looks_like_zip(data: &[u8]) -> bool {
    data.first() == Some(&b'P') && data.get(1) == Some(&b'K')
}

/// SAV version/kind live at bytes 3–6 (`SAV` + 2-digit version + 2-digit kind).
/// Kind `00` is plaintext; anything else is ironman/binary.
fn reject_binary(data: &[u8]) -> Result<(), ExportError> {
    if data.len() >= 7 && data.starts_with(b"SAV") && &data[5..7] != b"00" {
        return Err(ExportError::BinaryNotSupported);
    }
    Ok(())
}

fn read_zip_gamestate(data: &[u8]) -> Result<Vec<u8>, ExportError> {
    let mut archive = ZipArchive::new(Cursor::new(data)).map_err(zip_err)?;
    let mut file = match archive.by_name("gamestate") {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Err(ExportError::MissingGamestate),
        Err(err) => return Err(zip_err(err)),
    };
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|err| ExportError::Zip(err.to_string()))?;
    Ok(buf)
}

fn write_zip_gamestate(gamestate: &[u8]) -> Result<Vec<u8>, ExportError> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "gamestate",
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
    )
    .map_err(zip_err)?;
    zip.write_all(gamestate)
        .map_err(|err| ExportError::Zip(err.to_string()))?;
    let cursor = zip.finish().map_err(zip_err)?;
    Ok(cursor.into_inner())
}

fn zip_err(err: zip::result::ZipError) -> ExportError {
    let msg = err.to_string();
    if matches!(err, zip::result::ZipError::InvalidPassword)
        || msg.to_ascii_lowercase().contains("password")
    {
        ExportError::BinaryNotSupported
    } else {
        ExportError::Zip(msg)
    }
}

#[derive(Default)]
struct BuildingEdit {
    methods: Option<Vec<String>>,
    extra_levels: Option<u32>,
}

fn patch_gamestate(data: &[u8], patch: &SavePatch) -> Result<Vec<u8>, ExportError> {
    let mut edits: BTreeMap<u32, BuildingEdit> = BTreeMap::new();
    for item in &patch.production_methods {
        edits.entry(item.building_id).or_default().methods = Some(item.methods.clone());
    }
    for item in &patch.extra_levels {
        let edit = edits.entry(item.building_id).or_default();
        edit.extra_levels = Some(
            edit.extra_levels
                .unwrap_or(0)
                .saturating_add(item.extra_levels),
        );
    }

    let (db_open, db_close) = building_database_span(data)?;
    let inner = &data[db_open + 1..db_close];
    let entries = database_entries(inner)?;

    let mut replacements = Vec::new();
    for (id, edit) in &edits {
        let (_, open, close) = entries
            .iter()
            .copied()
            .find(|(entry_id, _, _)| entry_id == id)
            .ok_or(ExportError::BuildingNotFound(*id))?;
        let abs_open = db_open + 1 + open;
        let abs_close = db_open + 1 + close;
        let body = &data[abs_open + 1..abs_close];
        let patched_body = patch_building_body(body, *id, edit)?;
        replacements.push((abs_open + 1, abs_close, patched_body));
    }

    replacements.sort_by_key(|(start, _, _)| *start);
    let mut out = Vec::with_capacity(data.len());
    let mut cursor = 0;
    for (start, end, replacement) in replacements {
        out.extend_from_slice(&data[cursor..start]);
        out.extend_from_slice(&replacement);
        cursor = end;
    }
    out.extend_from_slice(&data[cursor..]);
    Ok(out)
}

fn building_database_span(data: &[u8]) -> Result<(usize, usize), ExportError> {
    let manager =
        find_object_value(data, b"building_manager").ok_or(ExportError::MissingBuildingManager)?;
    let inner = &data[manager.open + 1..manager.close];
    let db = find_object_value(inner, b"database")
        .or_else(|| find_object_value(inner, b"lod"))
        .ok_or(ExportError::MissingBuildingManager)?;
    Ok((manager.open + 1 + db.open, manager.open + 1 + db.close))
}

/// `(id, open, close)` of each `{ ... }` entry inside `database`, relative to `inner`.
fn database_entries(inner: &[u8]) -> Result<Vec<(u32, usize, usize)>, ExportError> {
    let mut entries = Vec::new();
    let mut pos = 0;
    while let Some(field) = parse_field(inner, &mut pos)? {
        let Ok(id) = std::str::from_utf8(field.key(inner))
            .unwrap_or("")
            .parse::<u32>()
        else {
            continue;
        };
        if let ValueSpan::Object { open, close } = field.value {
            entries.push((id, open, close));
        }
    }
    Ok(entries)
}

fn patch_building_body(
    body: &[u8],
    building_id: u32,
    edit: &BuildingEdit,
) -> Result<Vec<u8>, ExportError> {
    let mut replacements: Vec<(usize, usize, Vec<u8>)> = Vec::new();
    if let Some(methods) = edit.methods.as_deref() {
        replacements.push(methods_replacement(body, methods)?);
    }
    if let Some(extra) = edit.extra_levels {
        replacements.push(levels_replacement(body, building_id, extra)?);
    }
    replacements.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    let mut out = body.to_vec();
    for (start, end, replacement) in replacements {
        out.splice(start..end, replacement);
    }
    Ok(out)
}

fn methods_replacement(
    body: &[u8],
    methods: &[String],
) -> Result<(usize, usize, Vec<u8>), ExportError> {
    let formatted = format_methods(methods);
    let mut pos = 0;
    let mut found = None;
    let mut indent = Vec::from(&b"\t\t\t"[..]);
    if let Some(nl) = body.iter().position(|&b| b == b'\n') {
        let rest = &body[nl + 1..];
        let ws_len = rest
            .iter()
            .take_while(|b| **b == b' ' || **b == b'\t')
            .count();
        if ws_len > 0 {
            indent = rest[..ws_len].to_vec();
        }
    }
    while let Some(field) = parse_field(body, &mut pos)? {
        let key = field.key(body);
        if key == b"production_methods" || key == b"production_method" {
            found = Some((field.key_start, field.end));
            break;
        }
    }
    if let Some((start, end)) = found {
        return Ok((start, end, formatted));
    }
    let trim = body
        .iter()
        .rposition(|&b| !is_ws(b))
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut inserted = Vec::new();
    if trim > 0 && body.get(trim - 1) != Some(&b'\n') {
        inserted.push(b'\n');
    }
    inserted.extend_from_slice(&indent);
    inserted.extend_from_slice(&formatted);
    Ok((trim, body.len(), inserted))
}

fn levels_replacement(
    body: &[u8],
    building_id: u32,
    extra_levels: u32,
) -> Result<(usize, usize, Vec<u8>), ExportError> {
    let mut pos = 0;
    let mut found = None;
    while let Some(field) = parse_field(body, &mut pos)? {
        let key = field.key(body);
        if key == b"levels" || key == b"level" {
            found = Some(field);
            break;
        }
    }
    let field = found.ok_or(ExportError::MissingField {
        building_id,
        field: "levels",
    })?;
    let ValueSpan::Scalar { start, end } = field.value else {
        return Err(ExportError::MissingField {
            building_id,
            field: "levels",
        });
    };
    let current = parse_int(&body[start..end]).ok_or(ExportError::MissingField {
        building_id,
        field: "levels",
    })?;
    let extra = i32::try_from(extra_levels).map_err(|_| ExportError::Malformed)?;
    let next = current.checked_add(extra).ok_or(ExportError::Malformed)?;
    Ok((start, end, next.to_string().into_bytes()))
}

fn format_methods(methods: &[String]) -> Vec<u8> {
    let mut out = Vec::from(&b"production_methods={"[..]);
    if methods.is_empty() {
        out.extend_from_slice(b" }");
        return out;
    }
    for method in methods {
        out.push(b' ');
        out.push(b'"');
        out.extend_from_slice(method.as_bytes());
        out.push(b'"');
    }
    out.extend_from_slice(b" }");
    out
}

#[derive(Clone, Copy)]
struct ObjectSpan {
    open: usize,
    close: usize,
}

#[derive(Clone, Copy)]
enum ValueSpan {
    Object { open: usize, close: usize },
    Scalar { start: usize, end: usize },
}

struct Field {
    key_start: usize,
    key_end: usize,
    value: ValueSpan,
    end: usize,
}

impl Field {
    fn key<'a>(&self, data: &'a [u8]) -> &'a [u8] {
        &data[self.key_start..self.key_end]
    }
}

fn find_object_value(data: &[u8], key: &[u8]) -> Option<ObjectSpan> {
    let mut pos = 0;
    while let Ok(Some(field)) = parse_field(data, &mut pos) {
        if field.key(data) == key {
            if let ValueSpan::Object { open, close } = field.value {
                return Some(ObjectSpan { open, close });
            }
            return None;
        }
    }
    // Top-level files have a SAV header before the first key; skip scalars/junk
    // by scanning for an identifier match instead of requiring a single object.
    find_key_object(data, key)
}

fn find_key_object(data: &[u8], key: &[u8]) -> Option<ObjectSpan> {
    let mut i = 0;
    while i + key.len() < data.len() {
        if data[i..].starts_with(key)
            && (i == 0 || !is_ident(data[i - 1]))
            && !is_ident(data[i + key.len()])
        {
            let mut j = i + key.len();
            j = skip_ws(data, j);
            if data.get(j) == Some(&b'=') {
                j = skip_ws(data, j + 1);
                if data.get(j) == Some(&b'{') {
                    let close = matching_brace(data, j)?;
                    return Some(ObjectSpan { open: j, close });
                }
            }
        }
        i += 1;
    }
    None
}

fn parse_field(data: &[u8], pos: &mut usize) -> Result<Option<Field>, ExportError> {
    *pos = skip_ws(data, *pos);
    if *pos >= data.len() {
        return Ok(None);
    }
    let key_start = *pos;
    if !is_ident(data[key_start]) {
        return Ok(None);
    }
    let key_end = ident_end(data, key_start);
    *pos = skip_ws(data, key_end);
    if data.get(*pos) != Some(&b'=') {
        return Err(ExportError::Malformed);
    }
    *pos = skip_ws(data, *pos + 1);
    if *pos >= data.len() {
        return Err(ExportError::Malformed);
    }
    if data[*pos] == b'{' {
        let open = *pos;
        let close = matching_brace(data, open).ok_or(ExportError::Malformed)?;
        *pos = close + 1;
        return Ok(Some(Field {
            key_start,
            key_end,
            value: ValueSpan::Object { open, close },
            end: close + 1,
        }));
    }
    let start = *pos;
    if data[start] == b'"' {
        *pos = skip_quoted(data, start).ok_or(ExportError::Malformed)?;
    } else {
        while *pos < data.len() && !is_ws(data[*pos]) && data[*pos] != b'}' {
            *pos += 1;
        }
    }
    Ok(Some(Field {
        key_start,
        key_end,
        value: ValueSpan::Scalar { start, end: *pos },
        end: *pos,
    }))
}

fn matching_brace(data: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    let mut in_string = false;
    while i < data.len() {
        let b = data[i];
        if in_string {
            if b == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn skip_quoted(data: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < data.len() {
        match data[i] {
            b'\\' => i = i.saturating_add(2),
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn skip_ws(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() && is_ws(data[pos]) {
        pos += 1;
    }
    pos
}

fn ident_end(data: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < data.len() && is_ident(data[i]) {
        i += 1;
    }
    i
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn parse_int(bytes: &[u8]) -> Option<i32> {
    let s = std::str::from_utf8(bytes).ok()?.trim();
    let digits = s.split('.').next().unwrap_or(s);
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{empty_tokens, load_slice};

    fn tiny_save() -> Vec<u8> {
        br#"SAV01000000000000000000
building_manager={
	database={
		12345={
			building="building_rye_farm"
			levels=2
			production_methods={ "pm_simple_forestry" }
		}
		123456={
			building="building_logging_camp"
			levels=1
			production_methods={ "pm_simple_forestry" }
		}
		2=none
	}
}
"#
        .to_vec()
    }

    fn pm_patch(id: u32, methods: &[&str]) -> SavePatch {
        SavePatch {
            production_methods: vec![ProductionMethodPatch {
                building_id: id,
                methods: methods.iter().map(|s| (*s).to_string()).collect(),
            }],
            extra_levels: Vec::new(),
        }
    }

    #[test]
    fn patch_pms_are_visible_to_load_slice() {
        let original = tiny_save();
        let patched = export_save(
            &original,
            &pm_patch(12345, &["pm_soil_enriching_farming", "pm_no_automation"]),
        )
        .expect("patch");
        let save = load_slice(&patched, empty_tokens()).expect("load patched");
        let building = save
            .building_manager
            .database
            .get(&12345)
            .and_then(|slot| slot.as_ref())
            .expect("building 12345");
        assert_eq!(
            building.active_production_methods(),
            ["pm_soil_enriching_farming", "pm_no_automation"]
        );
        assert_eq!(building.level, 2);
    }

    #[test]
    fn original_bytes_are_not_written() {
        let original = tiny_save();
        let before = original.clone();
        let patched = export_save(&original, &pm_patch(12345, &["pm_a"])).expect("patch");
        assert_eq!(original, before, "caller slice must stay intact");
        assert_ne!(patched, original);
        assert!(
            !std::str::from_utf8(&original).unwrap().contains("pm_a"),
            "origin must not contain the patched methods"
        );
    }

    #[test]
    fn binary_sav_header_is_rejected() {
        let mut bytes = b"SAV0101deadbeef00000000\n".to_vec();
        bytes.extend_from_slice(&[0u8; 32]);
        let err = export_save(&bytes, &pm_patch(1, &["pm_a"])).expect_err("binary");
        let msg = err.to_string();
        assert!(
            msg.to_ascii_lowercase().contains("ironman")
                && msg.to_ascii_lowercase().contains("binary"),
            "error should mention ironman/binary not supported, got: {msg}"
        );
        assert!(msg.to_ascii_lowercase().contains("not supported"), "{msg}");
    }

    #[test]
    fn missing_building_id_errors() {
        let err = export_save(&tiny_save(), &pm_patch(999, &["pm_a"])).expect_err("missing");
        assert!(matches!(err, ExportError::BuildingNotFound(999)));
    }

    #[test]
    fn extra_levels_add_to_saved_level() {
        let patch = SavePatch {
            production_methods: Vec::new(),
            extra_levels: vec![ExtraLevelsPatch {
                building_id: 12345,
                extra_levels: 3,
            }],
        };
        let patched = export_save(&tiny_save(), &patch).expect("patch");
        let save = load_slice(&patched, empty_tokens()).expect("load");
        let building = save
            .building_manager
            .database
            .get(&12345)
            .and_then(|slot| slot.as_ref())
            .unwrap();
        assert_eq!(building.level, 5);
    }

    #[test]
    fn building_id_is_not_a_prefix_match() {
        let patched = export_save(&tiny_save(), &pm_patch(12345, &["pm_only"])).expect("patch");
        let text = String::from_utf8(patched).unwrap();
        assert!(text.contains("12345={"));
        assert!(text.contains("pm_only"));
        assert!(text.contains("123456={"));
        assert!(
            text.contains("building_logging_camp") && text.contains("pm_simple_forestry"),
            "longer id 123456 must keep its original methods: {text}"
        );
    }

    #[test]
    fn zip_input_returns_zip_with_gamestate_member() {
        let raw = tiny_save();
        let zipped = write_zip_gamestate(&raw).expect("fixture zip");
        assert!(looks_like_zip(&zipped));
        let patched = export_save(&zipped, &pm_patch(12345, &["pm_zip"])).expect("patch zip");
        assert!(looks_like_zip(&patched));
        let gamestate = read_zip_gamestate(&patched).expect("unzip");
        let save = load_slice(&gamestate, empty_tokens()).expect("load");
        let building = save
            .building_manager
            .database
            .get(&12345)
            .and_then(|slot| slot.as_ref())
            .unwrap();
        assert_eq!(building.active_production_methods(), ["pm_zip"]);
    }

    #[test]
    fn empty_patch_returns_a_copy() {
        let original = tiny_save();
        let out = export_save(&original, &SavePatch::default()).expect("empty");
        assert_eq!(out, original);
    }
}
