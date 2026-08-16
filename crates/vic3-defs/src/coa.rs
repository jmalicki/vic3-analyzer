//! Minimal coat-of-arms parse + render for country flags.
//!
//! Full Clausewitz heraldry (masks, cantons, every trigger) is out of scope.
//! This renders a small PNG from `pattern` + `color1` and optional centered
//! colored/textured emblems so foreign states can show a recognizable flag.

use std::collections::BTreeMap;

use crate::icons;

const FLAG_W: u32 = 64;
const FLAG_H: u32 = 42;

#[derive(Debug, Clone, Default)]
pub struct CoaLibrary {
    pub colors: BTreeMap<String, [u8; 4]>,
    pub coats: BTreeMap<String, CoatOfArms>,
    /// `tag` → prioritized flag definitions (coa id + optional law trigger).
    pub flag_defs: BTreeMap<String, Vec<FlagDef>>,
    /// Rendered CoA id → PNG bytes.
    pub rendered: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct CoatOfArms {
    pub pattern: Option<String>,
    pub color1: Option<String>,
    pub color2: Option<String>,
    pub color3: Option<String>,
    pub emblems: Vec<Emblem>,
}

#[derive(Debug, Clone)]
pub struct Emblem {
    pub texture: String,
    pub color1: Option<String>,
    pub colored: bool,
}

#[derive(Debug, Clone)]
pub struct FlagDef {
    pub coa: String,
    pub priority: i32,
    /// Empty = always eligible. Otherwise a single `has_law_or_variant = law_type:X`
    /// (or OR of those) we can evaluate. Unsupported triggers leave this as
    /// [`FlagTrigger::Unsupported`].
    pub trigger: FlagTrigger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagTrigger {
    Always,
    /// Country must have at least one of these law type ids.
    AnyLaw(Vec<String>),
    Unsupported,
}

/// Select the current CoA id for a country tag given enacted laws.
pub fn select_flag_coa(
    flag_defs: &BTreeMap<String, Vec<crate::FlagDefinition>>,
    flags: &BTreeMap<String, Vec<u8>>,
    tag: &str,
    laws: &[String],
) -> Option<String> {
    let mut best: Option<&crate::FlagDefinition> = None;
    for defs in [flag_defs.get(tag), flag_defs.get("DEFAULT")]
        .into_iter()
        .flatten()
    {
        for def in defs {
            if def.unsupported_trigger {
                continue;
            }
            let ok = if def.any_laws.is_empty() {
                true
            } else {
                def.any_laws.iter().any(|law| {
                    laws.iter()
                        .any(|have| have == law || have.ends_with(law.as_str()))
                })
            };
            if ok && best.is_none_or(|b| def.priority >= b.priority) {
                best = Some(def);
            }
        }
    }
    if let Some(def) = best {
        if flags.contains_key(&def.coa) {
            return Some(def.coa.clone());
        }
    }
    if flags.contains_key(tag) {
        return Some(tag.to_string());
    }
    None
}

/// Select the current CoA id for a country tag given enacted laws.
///
/// Definitions whose trigger we cannot evaluate are skipped (never silently
/// substituted). Tag-specific defs compete with the shared `DEFAULT` list.
/// If nothing matches, fall back to a CoA named after the tag when one exists
/// in the library.
pub fn select_coa(library: &CoaLibrary, tag: &str, laws: &[String]) -> Option<String> {
    let mut best: Option<&FlagDef> = None;
    for defs in [library.flag_defs.get(tag), library.flag_defs.get("DEFAULT")]
        .into_iter()
        .flatten()
    {
        for def in defs {
            let ok = match &def.trigger {
                FlagTrigger::Always => true,
                FlagTrigger::AnyLaw(needed) => needed.iter().any(|law| {
                    laws.iter()
                        .any(|have| have == law || have.ends_with(law.as_str()))
                }),
                FlagTrigger::Unsupported => false,
            };
            if ok && best.is_none_or(|b| def.priority >= b.priority) {
                best = Some(def);
            }
        }
    }
    if let Some(def) = best {
        return Some(def.coa.clone());
    }
    if library.coats.contains_key(tag) {
        return Some(tag.to_string());
    }
    None
}

pub fn parse_named_colors(bytes: &[u8], into: &mut BTreeMap<String, [u8; 4]>) {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
            continue;
        }
        // name = rgb { r g b } or hsv / hsv360
        let Some((name, rest)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim().to_string();
        let rest = rest.trim();
        if let Some(rgb) = parse_rgb(rest) {
            into.insert(name, rgb);
        }
    }
}

fn parse_rgb(rest: &str) -> Option<[u8; 4]> {
    let rest = rest.trim();
    if let Some(inner) = rest.strip_prefix("rgb") {
        let inner = inner
            .trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim();
        let parts: Vec<f64> = inner
            .split_whitespace()
            .filter_map(|p| p.parse().ok())
            .collect();
        if parts.len() >= 3 {
            return Some([
                parts[0].clamp(0.0, 255.0) as u8,
                parts[1].clamp(0.0, 255.0) as u8,
                parts[2].clamp(0.0, 255.0) as u8,
                255,
            ]);
        }
    }
    None
}

/// Parse one coat-of-arms definitions file into `coats`.
pub fn parse_coat_of_arms_file(bytes: &[u8], coats: &mut BTreeMap<String, CoatOfArms>) {
    let text = String::from_utf8_lossy(bytes);
    let mut id = String::new();
    let mut current = CoatOfArms::default();
    let mut depth = 0i32;
    let mut in_emblem = false;
    let mut emblem_colored = false;
    let mut emblem = Emblem {
        texture: String::new(),
        color1: None,
        colored: false,
    };

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
            continue;
        }
        if depth == 0 {
            if let Some((name, rest)) = line.split_once('=') {
                if rest.trim().starts_with('{') {
                    id = name.trim().to_string();
                    current = CoatOfArms::default();
                    depth = 1;
                    continue;
                }
            }
        }
        if line.contains('{') {
            depth += line.matches('{').count() as i32;
        }
        if line.starts_with("colored_emblem") {
            in_emblem = true;
            emblem_colored = true;
            emblem = Emblem {
                texture: String::new(),
                color1: None,
                colored: true,
            };
        } else if line.starts_with("textured_emblem") {
            in_emblem = true;
            emblem_colored = false;
            emblem = Emblem {
                texture: String::new(),
                color1: None,
                colored: false,
            };
        } else if in_emblem {
            if let Some(tex) = line.strip_prefix("texture") {
                let tex = tex.trim().trim_start_matches('=').trim().trim_matches('"');
                emblem.texture = tex.to_string();
                emblem.colored = emblem_colored;
            } else if let Some(c) = line.strip_prefix("color1") {
                let c = c.trim().trim_start_matches('=').trim().trim_matches('"');
                emblem.color1 = Some(c.to_string());
            }
        } else if depth == 1 {
            if let Some(rest) = line.strip_prefix("pattern") {
                let v = rest.trim().trim_start_matches('=').trim().trim_matches('"');
                current.pattern = Some(v.to_string());
            } else if let Some(rest) = line.strip_prefix("color1") {
                let v = rest.trim().trim_start_matches('=').trim().trim_matches('"');
                current.color1 = Some(v.to_string());
            } else if let Some(rest) = line.strip_prefix("color2") {
                let v = rest.trim().trim_start_matches('=').trim().trim_matches('"');
                current.color2 = Some(v.to_string());
            } else if let Some(rest) = line.strip_prefix("color3") {
                let v = rest.trim().trim_start_matches('=').trim().trim_matches('"');
                current.color3 = Some(v.to_string());
            }
        }
        if line.contains('}') {
            let closes = line.matches('}').count() as i32;
            if in_emblem && closes > 0 {
                if !emblem.texture.is_empty() {
                    current.emblems.push(emblem.clone());
                }
                in_emblem = false;
            }
            depth -= closes;
            if depth <= 0 && !id.is_empty() {
                coats.insert(id.clone(), current.clone());
                id.clear();
                depth = 0;
            }
        }
    }
}

/// Parse flag_definitions files.
pub fn parse_flag_definitions(bytes: &[u8], into: &mut BTreeMap<String, Vec<FlagDef>>) {
    let text = String::from_utf8_lossy(bytes);
    let mut tag = String::new();
    let mut depth = 0i32;
    let mut in_def = false;
    let mut coa = String::new();
    let mut priority = 0i32;
    let mut trigger = FlagTrigger::Always;
    let mut trigger_depth = 0i32;
    let mut laws = Vec::new();
    let mut unsupported = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
            continue;
        }
        if depth == 0 {
            if let Some((name, rest)) = line.split_once('=') {
                if rest.trim().starts_with('{') {
                    tag = name.trim().to_string();
                    depth = 1;
                    continue;
                }
            }
        }
        if line.starts_with("flag_definition") {
            in_def = true;
            coa.clear();
            priority = 0;
            trigger = FlagTrigger::Always;
            laws.clear();
            unsupported = false;
            trigger_depth = 0;
        }
        if line.contains('{') {
            depth += line.matches('{').count() as i32;
            if line.contains("trigger") {
                trigger_depth = 1;
            } else if trigger_depth > 0 {
                trigger_depth += line.matches('{').count() as i32;
            }
        }
        if in_def {
            if trigger_depth > 0 {
                if line.contains("has_law_or_variant") {
                    if let Some(idx) = line.find("law_type:") {
                        let law = line[idx + "law_type:".len()..]
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                            .to_string();
                        if !law.is_empty() {
                            laws.push(law);
                        }
                    } else {
                        unsupported = true;
                    }
                } else if line.contains('=')
                    && !line.starts_with("OR")
                    && !line.starts_with("AND")
                    && !line.starts_with("NOT")
                    && !line.starts_with("trigger")
                    && !line.contains("has_law")
                    && !line.starts_with('{')
                    && !line.starts_with('}')
                {
                    // other atoms inside trigger
                    if !line.contains("exists") && !line.contains("scope:") {
                        unsupported = true;
                    }
                }
            } else if let Some(rest) = line.strip_prefix("coa") {
                if !line.starts_with("coa_with") && !line.starts_with("coa_") {
                    let v = rest.trim().trim_start_matches('=').trim();
                    // `coa = PRU` or `coa = list "communist"`
                    let v = if let Some(list) = v.strip_prefix("list") {
                        list.trim().trim_matches('"').to_string()
                    } else {
                        v.trim_matches('"').to_string()
                    };
                    if !v.is_empty() {
                        coa = v;
                    }
                }
            } else if let Some(rest) = line.strip_prefix("priority") {
                let v = rest.trim().trim_start_matches('=').trim();
                priority = v.parse().unwrap_or(0);
            }
        }
        if line.contains('}') {
            let closes = line.matches('}').count() as i32;
            if trigger_depth > 0 {
                trigger_depth -= closes;
                if trigger_depth <= 0 {
                    trigger_depth = 0;
                    trigger = if unsupported {
                        FlagTrigger::Unsupported
                    } else if laws.is_empty() {
                        FlagTrigger::Always
                    } else {
                        FlagTrigger::AnyLaw(laws.clone())
                    };
                }
            }
            depth -= closes;
            if in_def && depth <= 1 && line.contains('}') {
                // end of flag_definition block roughly when depth back to 1
                if depth == 1 && !coa.is_empty() {
                    into.entry(tag.clone()).or_default().push(FlagDef {
                        coa: coa.clone(),
                        priority,
                        trigger: trigger.clone(),
                    });
                    in_def = false;
                    coa.clear();
                }
            }
            if depth <= 0 {
                tag.clear();
                depth = 0;
                in_def = false;
            }
        }
    }
}

/// Render coats that have textures available into PNG.
pub fn render_library(
    library: &mut CoaLibrary,
    textures: &BTreeMap<String, Vec<u8>>, // filename stem/name → RGBA via dds or raw
) {
    let coats = library.coats.clone();
    for (id, coa) in coats {
        if let Some(png) = render_coa(&coa, &library.colors, textures) {
            library.rendered.insert(id, png);
        }
    }
}

fn resolve_color(colors: &BTreeMap<String, [u8; 4]>, name: Option<&String>) -> [u8; 4] {
    name.and_then(|n| colors.get(n).copied())
        .unwrap_or([200, 0, 200, 255])
}

fn render_coa(
    coa: &CoatOfArms,
    colors: &BTreeMap<String, [u8; 4]>,
    textures: &BTreeMap<String, Vec<u8>>,
) -> Option<Vec<u8>> {
    let fill = resolve_color(colors, coa.color1.as_ref());
    let mut rgba = vec![0u8; (FLAG_W * FLAG_H * 4) as usize];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&fill);
    }

    // If we have a pattern texture, recolor by R channel → color1 (solid patterns).
    if let Some(pattern) = &coa.pattern {
        let key = texture_key(pattern);
        if let Some(tex) = textures.get(&key) {
            if let Some(img) = decode_texture(tex) {
                blit_recolor(&mut rgba, &img, resolve_color(colors, coa.color1.as_ref()));
            }
        }
    }

    for emblem in &coa.emblems {
        let key = texture_key(&emblem.texture);
        if let Some(tex) = textures.get(&key) {
            if let Some(img) = decode_texture(tex) {
                let tint = if emblem.colored {
                    resolve_color(colors, emblem.color1.as_ref().or(coa.color1.as_ref()))
                } else {
                    [255, 255, 255, 255]
                };
                blit_centered(&mut rgba, &img, tint, emblem.colored);
            }
        }
    }

    encode_png(FLAG_W, FLAG_H, &rgba)
}

fn texture_key(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_lowercase()
}

struct RgbaImage {
    w: u32,
    h: u32,
    data: Vec<u8>,
}

fn decode_texture(bytes: &[u8]) -> Option<RgbaImage> {
    if bytes.starts_with(b"DDS ") {
        let png = icons::dds_to_png(bytes)?;
        return decode_png(&png);
    }
    if bytes.starts_with(b"\x89PNG") {
        return decode_png(bytes);
    }
    // TGA: skip for now unless uncompressed — many patterns are TGA.
    decode_tga(bytes)
}

fn decode_png(bytes: &[u8]) -> Option<RgbaImage> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    if buf.is_empty() {
        return None;
    }
    let info = reader.next_frame(&mut buf).ok()?;
    let mut data = buf[..info.buffer_size()].to_vec();
    // Expand to RGBA if needed
    match info.color_type {
        png::ColorType::Rgba => {}
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(data.len() / 3 * 4);
            for chunk in data.chunks_exact(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            data = rgba;
        }
        _ => return None,
    }
    Some(RgbaImage {
        w: info.width,
        h: info.height,
        data,
    })
}

fn decode_tga(bytes: &[u8]) -> Option<RgbaImage> {
    if bytes.len() < 18 {
        return None;
    }
    let w = u16::from_le_bytes([bytes[12], bytes[13]]) as u32;
    let h = u16::from_le_bytes([bytes[14], bytes[15]]) as u32;
    let bpp = bytes[16];
    let img_type = bytes[2];
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        return None;
    }
    // Uncompressed true-color
    if img_type != 2 || (bpp != 24 && bpp != 32) {
        return None;
    }
    let id_len = bytes[0] as usize;
    let start = 18 + id_len;
    let px = (bpp / 8) as usize;
    let need = start + (w * h) as usize * px;
    if bytes.len() < need {
        return None;
    }
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        // TGA is often bottom-up
        let row = h - 1 - y;
        for x in 0..w {
            let i = start + ((row * w + x) as usize) * px;
            let b = bytes[i];
            let g = bytes[i + 1];
            let r = bytes[i + 2];
            let a = if px == 4 { bytes[i + 3] } else { 255 };
            data.extend_from_slice(&[r, g, b, a]);
        }
    }
    Some(RgbaImage { w, h, data })
}

fn blit_recolor(canvas: &mut [u8], img: &RgbaImage, color: [u8; 4]) {
    for y in 0..FLAG_H {
        for x in 0..FLAG_W {
            let sx = x * img.w / FLAG_W;
            let sy = y * img.h / FLAG_H;
            let si = ((sy * img.w + sx) * 4) as usize;
            let r = img.data.get(si).copied().unwrap_or(0) as f64 / 255.0;
            let a = img.data.get(si + 3).copied().unwrap_or(255);
            let di = ((y * FLAG_W + x) * 4) as usize;
            if a == 0 {
                continue;
            }
            canvas[di] = (color[0] as f64 * r) as u8;
            canvas[di + 1] = (color[1] as f64 * r) as u8;
            canvas[di + 2] = (color[2] as f64 * r) as u8;
            canvas[di + 3] = 255;
        }
    }
}

fn blit_centered(canvas: &mut [u8], img: &RgbaImage, tint: [u8; 4], recolor: bool) {
    let dw = FLAG_W * 3 / 4;
    let dh = FLAG_H * 3 / 4;
    let ox = (FLAG_W - dw) / 2;
    let oy = (FLAG_H - dh) / 2;
    for y in 0..dh {
        for x in 0..dw {
            let sx = x * img.w / dw;
            let sy = y * img.h / dh;
            let si = ((sy * img.w + sx) * 4) as usize;
            let sr = img.data.get(si).copied().unwrap_or(0);
            let sg = img.data.get(si + 1).copied().unwrap_or(0);
            let sb = img.data.get(si + 2).copied().unwrap_or(0);
            let sa = img.data.get(si + 3).copied().unwrap_or(0);
            if sa < 8 {
                continue;
            }
            let (r, g, b) = if recolor {
                let w = sr as f64 / 255.0;
                (
                    (tint[0] as f64 * w) as u8,
                    (tint[1] as f64 * w) as u8,
                    (tint[2] as f64 * w) as u8,
                )
            } else {
                (sr, sg, sb)
            };
            let di = (((oy + y) * FLAG_W + (ox + x)) * 4) as usize;
            let alpha = sa as f64 / 255.0;
            canvas[di] = ((r as f64 * alpha) + canvas[di] as f64 * (1.0 - alpha)) as u8;
            canvas[di + 1] = ((g as f64 * alpha) + canvas[di + 1] as f64 * (1.0 - alpha)) as u8;
            canvas[di + 2] = ((b as f64 * alpha) + canvas[di + 2] as f64 * (1.0 - alpha)) as u8;
            canvas[di + 3] = 255;
        }
    }
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    drop(writer);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_coa_prefers_matching_law_over_always() {
        let mut library = CoaLibrary::default();
        library.flag_defs.insert(
            "PRU".into(),
            vec![
                FlagDef {
                    coa: "PRU".into(),
                    priority: 1,
                    trigger: FlagTrigger::Always,
                },
                FlagDef {
                    coa: "PRU_republic".into(),
                    priority: 10,
                    trigger: FlagTrigger::AnyLaw(vec!["law_presidential_republic".into()]),
                },
            ],
        );
        library.coats.insert("PRU".into(), CoatOfArms::default());
        library
            .coats
            .insert("PRU_republic".into(), CoatOfArms::default());

        assert_eq!(
            select_coa(&library, "PRU", &["law_presidential_republic".into()]).as_deref(),
            Some("PRU_republic")
        );
        assert_eq!(select_coa(&library, "PRU", &[]).as_deref(), Some("PRU"));
    }

    #[test]
    fn unsupported_trigger_is_not_selected() {
        let mut library = CoaLibrary::default();
        library.flag_defs.insert(
            "X".into(),
            vec![FlagDef {
                coa: "X_special".into(),
                priority: 100,
                trigger: FlagTrigger::Unsupported,
            }],
        );
        library.coats.insert("X".into(), CoatOfArms::default());
        assert_eq!(select_coa(&library, "X", &[]).as_deref(), Some("X"));
    }

    #[test]
    fn default_list_competes_with_tag_defs() {
        let mut library = CoaLibrary::default();
        library.flag_defs.insert(
            "TST".into(),
            vec![FlagDef {
                coa: "TST".into(),
                priority: 1,
                trigger: FlagTrigger::Always,
            }],
        );
        library.flag_defs.insert(
            "DEFAULT".into(),
            vec![FlagDef {
                coa: "communist".into(),
                priority: 1000,
                trigger: FlagTrigger::AnyLaw(vec!["law_council_republic".into()]),
            }],
        );
        assert_eq!(select_coa(&library, "TST", &[]).as_deref(), Some("TST"));
        assert_eq!(
            select_coa(&library, "TST", &["law_council_republic".into()]).as_deref(),
            Some("communist")
        );
    }

    #[test]
    fn parses_list_coa_syntax() {
        let mut defs = BTreeMap::new();
        parse_flag_definitions(
            br#"
TAG = {
	flag_definition = {
		coa = list "communist"
		priority = 10
	}
}
"#,
            &mut defs,
        );
        assert_eq!(defs["TAG"][0].coa, "communist");
    }

    #[test]
    fn solid_color_coa_renders_png() {
        let mut colors = BTreeMap::new();
        colors.insert("white".into(), [255, 255, 255, 255]);
        let coa = CoatOfArms {
            color1: Some("white".into()),
            ..CoatOfArms::default()
        };
        let png = render_coa(&coa, &colors, &BTreeMap::new()).expect("png");
        assert!(png.starts_with(b"\x89PNG"));
    }
}
