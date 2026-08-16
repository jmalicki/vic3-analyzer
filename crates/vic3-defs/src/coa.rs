//! Minimal coat-of-arms parse + render for country flags.
//!
//! Full Clausewitz heraldry (masks, cantons, every trigger) is out of scope.
//! This renders a small PNG from `pattern` + `color1` and optional centered
//! colored/textured emblems so foreign states can show a recognizable flag.

use std::collections::{BTreeMap, BTreeSet};

use crate::icons;

const FLAG_W: u32 = 64;
const FLAG_H: u32 = 42;

#[derive(Debug, Clone, Default)]
pub struct CoaLibrary {
    pub colors: BTreeMap<String, [u8; 4]>,
    pub coats: BTreeMap<String, CoatOfArms>,
    /// Template-list id → concrete CoA ids. Only unambiguous lists are
    /// resolved; randomized lists deliberately remain unavailable.
    pub template_lists: BTreeMap<String, Vec<String>>,
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
    pub color4: Option<String>,
    pub emblems: Vec<Emblem>,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Emblem {
    pub texture: String,
    pub color1: Option<String>,
    pub color2: Option<String>,
    pub color3: Option<String>,
    pub colored: bool,
    pub instances: Vec<EmblemInstance>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmblemInstance {
    pub position: [f64; 2],
    pub scale: [f64; 2],
}

impl Default for EmblemInstance {
    fn default() -> Self {
        Self {
            position: [0.5, 0.5],
            scale: [1.0, 1.0],
        }
    }
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
            if ok && flags.contains_key(&def.coa) && best.is_none_or(|b| def.priority >= b.priority)
            {
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
    for entry in entries(&text) {
        let color_entries = if entry.key == "colors" {
            entries(&entry.value)
        } else {
            vec![entry]
        };
        for color in color_entries {
            if let Some(rgba) = parse_color(&color.value) {
                into.insert(color.key, rgba);
            }
        }
    }
}

fn parse_color(raw: &str) -> Option<[u8; 4]> {
    let raw = raw.trim();
    let lower = raw.to_ascii_lowercase();
    if let Some(hex) = lower
        .strip_prefix("hex")
        .map(str::trim)
        .or_else(|| lower.strip_prefix('#'))
    {
        let hex = unquote(hex)
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim()
            .trim_start_matches('#');
        if hex.len() == 6 || hex.len() == 8 {
            let value = u32::from_str_radix(hex, 16).ok()?;
            return Some(if hex.len() == 8 {
                [
                    (value >> 24) as u8,
                    (value >> 16) as u8,
                    (value >> 8) as u8,
                    value as u8,
                ]
            } else {
                [(value >> 16) as u8, (value >> 8) as u8, value as u8, 255]
            });
        }
    }
    let (mode, values) = if let Some(rest) = lower.strip_prefix("hsv360") {
        ("hsv360", numbers(rest))
    } else if let Some(rest) = lower.strip_prefix("hsv") {
        ("hsv", numbers(rest))
    } else if let Some(rest) = lower.strip_prefix("rgb") {
        ("rgb", numbers(rest))
    } else {
        ("rgb", numbers(&lower))
    };
    let [a, b, c, ..] = values.as_slice() else {
        return None;
    };
    match mode {
        "hsv360" => Some(hsv_to_rgba(*a / 360.0, *b / 100.0, *c / 100.0)),
        "hsv" => Some(hsv_to_rgba(*a, *b, *c)),
        _ => {
            let scale = if [*a, *b, *c].iter().all(|value| *value <= 1.0) {
                255.0
            } else {
                1.0
            };
            Some([
                (a * scale).clamp(0.0, 255.0).round() as u8,
                (b * scale).clamp(0.0, 255.0).round() as u8,
                (c * scale).clamp(0.0, 255.0).round() as u8,
                255,
            ])
        }
    }
}

fn hsv_to_rgba(hue: f64, saturation: f64, value: f64) -> [u8; 4] {
    let h = hue.rem_euclid(1.0) * 6.0;
    let s = saturation.clamp(0.0, 1.0);
    let v = value.clamp(0.0, 1.0);
    let chroma = v * s;
    let x = chroma * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = v - chroma;
    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
        255,
    ]
}

/// Parse one coat-of-arms definitions file into `coats`.
pub fn parse_coat_of_arms_file(bytes: &[u8], coats: &mut BTreeMap<String, CoatOfArms>) {
    let text = String::from_utf8_lossy(bytes);
    for entry in entries(&text) {
        let definitions = if entry.key == "template" {
            entries(&entry.value)
        } else {
            vec![entry]
        };
        for definition in definitions {
            if let Some(coa) = parse_coa_body(&definition.value) {
                coats.insert(definition.key, coa);
            }
        }
    }
}

pub fn parse_template_lists(bytes: &[u8], into: &mut BTreeMap<String, Vec<String>>) {
    let text = String::from_utf8_lossy(bytes);
    for root in entries(&text) {
        if !root.key.ends_with("_lists") {
            continue;
        }
        for list in entries(&root.value) {
            let candidates = entries(&list.value)
                .into_iter()
                .filter(|entry| entry.key.parse::<u32>().is_ok())
                .map(|entry| unquote(&entry.value).to_string())
                .collect::<Vec<_>>();
            if !candidates.is_empty() {
                into.entry(list.key).or_default().extend(candidates);
            }
        }
    }
}

fn parse_coa_body(body: &str) -> Option<CoatOfArms> {
    let mut coa = CoatOfArms::default();
    for entry in entries(body) {
        match entry.key.as_str() {
            "pattern" => coa.pattern = scalar_value(&entry.value),
            "color1" => coa.color1 = scalar_value(&entry.value),
            "color2" => coa.color2 = scalar_value(&entry.value),
            "color3" => coa.color3 = scalar_value(&entry.value),
            "color4" => coa.color4 = scalar_value(&entry.value),
            "colored_emblem" | "textured_emblem" => {
                if let Some(emblem) = parse_emblem(&entry.value, entry.key == "colored_emblem") {
                    coa.emblems.push(emblem);
                }
            }
            "sub" => {
                if let Some(parent) = entries(&entry.value)
                    .into_iter()
                    .find(|field| field.key == "parent")
                    .and_then(|field| scalar_value(&field.value))
                {
                    coa.parents.push(parent);
                }
            }
            _ => {}
        }
    }
    (coa.pattern.is_some() || !coa.emblems.is_empty() || !coa.parents.is_empty()).then_some(coa)
}

fn parse_emblem(body: &str, colored: bool) -> Option<Emblem> {
    let mut emblem = Emblem {
        texture: String::new(),
        color1: None,
        color2: None,
        color3: None,
        colored,
        instances: Vec::new(),
    };
    for entry in entries(body) {
        match entry.key.as_str() {
            "texture" => emblem.texture = scalar_value(&entry.value)?,
            "color1" => emblem.color1 = scalar_value(&entry.value),
            "color2" => emblem.color2 = scalar_value(&entry.value),
            "color3" => emblem.color3 = scalar_value(&entry.value),
            "instance" => emblem.instances.push(parse_instance(&entry.value)),
            _ => {}
        }
    }
    (!emblem.texture.is_empty()).then_some(emblem)
}

fn parse_instance(body: &str) -> EmblemInstance {
    let mut instance = EmblemInstance::default();
    for entry in entries(body) {
        match entry.key.as_str() {
            "position" | "offset" => {
                if let Some(value) = pair(&entry.value) {
                    instance.position = value;
                }
            }
            "scale" => {
                if let Some(value) = pair(&entry.value) {
                    instance.scale = value;
                }
            }
            _ => {}
        }
    }
    instance
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
    resolve_parents(&mut library.coats);
    resolve_template_lists(library);
    let decoded = library
        .needed_texture_names()
        .into_iter()
        .filter_map(|key: String| Some((key.clone(), decode_flag_texture(textures.get(&key)?)?)))
        .collect::<BTreeMap<_, _>>();
    render_resolved(library, &decoded);
}

/// Render from textures already decoded to flag size.
///
/// The browser builder scales each texture as it arrives so the source art —
/// over 400 MB across a full install — never has to be held at once.
pub fn render_library_scaled(library: &mut CoaLibrary, textures: &BTreeMap<String, RgbaImage>) {
    resolve_parents(&mut library.coats);
    resolve_template_lists(library);
    render_resolved(library, textures);
}

fn render_resolved(library: &mut CoaLibrary, textures: &BTreeMap<String, RgbaImage>) {
    let coats = library.coats.clone();
    for (id, coa) in coats {
        if let Some(png) = render_coa(&coa, &library.colors, textures) {
            library.rendered.insert(id, png);
        }
    }
}

impl CoaLibrary {
    /// Texture file names every coat could ask for.
    ///
    /// Parent and template-list resolution only copy emblems between coats
    /// already present, so the union taken before resolving is a superset of
    /// what rendering will look up.
    pub fn needed_texture_names(&self) -> BTreeSet<String> {
        self.coats
            .values()
            .flat_map(|coat| {
                coat.pattern
                    .iter()
                    .chain(coat.emblems.iter().map(|emblem| &emblem.texture))
            })
            .map(|texture| texture_key(texture))
            .collect()
    }
}

/// Decode one texture and immediately reduce it to flag size.
pub(crate) fn decode_flag_texture(bytes: &[u8]) -> Option<RgbaImage> {
    Some(scale_to_flag(&decode_texture(bytes)?))
}

fn resolve_parents(coats: &mut BTreeMap<String, CoatOfArms>) {
    let source = coats.clone();
    let ids = source.keys().cloned().collect::<Vec<_>>();
    for id in ids {
        let mut visiting = Vec::new();
        if let Some(resolved) = resolve_parent(&id, &source, &mut visiting) {
            coats.insert(id, resolved);
        }
    }
}

fn resolve_parent(
    id: &str,
    coats: &BTreeMap<String, CoatOfArms>,
    visiting: &mut Vec<String>,
) -> Option<CoatOfArms> {
    if visiting.iter().any(|item| item == id) {
        return None;
    }
    let mut coat = coats.get(id)?.clone();
    visiting.push(id.to_string());
    let parents = std::mem::take(&mut coat.parents);
    for parent_id in parents {
        let parent = resolve_parent(&parent_id, coats, visiting)?;
        if coat.pattern.is_none() {
            coat.pattern = parent.pattern;
        }
        if coat.color1.is_none() {
            coat.color1 = parent.color1;
        }
        if coat.color2.is_none() {
            coat.color2 = parent.color2;
        }
        if coat.color3.is_none() {
            coat.color3 = parent.color3;
        }
        if coat.color4.is_none() {
            coat.color4 = parent.color4;
        }
        let mut inherited = parent.emblems;
        inherited.append(&mut coat.emblems);
        coat.emblems = inherited;
    }
    visiting.pop();
    Some(coat)
}

fn resolve_template_lists(library: &mut CoaLibrary) {
    for (list, candidates) in library.template_lists.clone() {
        let unique = candidates
            .into_iter()
            .filter(|candidate| library.coats.contains_key(candidate))
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() == 1 {
            let target = unique.into_iter().next().expect("one candidate");
            if let Some(coat) = library.coats.get(&target).cloned() {
                library.coats.insert(list, coat);
            }
        }
    }
}

fn resolve_color(
    colors: &BTreeMap<String, [u8; 4]>,
    coa: &CoatOfArms,
    name: Option<&String>,
) -> Option<[u8; 4]> {
    let name = name?;
    let referenced = match name.as_str() {
        "color1" => coa.color1.as_ref(),
        "color2" => coa.color2.as_ref(),
        "color3" => coa.color3.as_ref(),
        "color4" => coa.color4.as_ref(),
        _ => None,
    };
    if let Some(referenced) = referenced {
        if referenced == name {
            return None;
        }
        return resolve_color(colors, coa, Some(referenced));
    }
    colors.get(name).copied().or_else(|| parse_color(name))
}

fn render_coa(
    coa: &CoatOfArms,
    colors: &BTreeMap<String, [u8; 4]>,
    textures: &BTreeMap<String, RgbaImage>,
) -> Option<Vec<u8>> {
    let fill = resolve_color(colors, coa, coa.color1.as_ref())?;
    let mut rgba = vec![0u8; (FLAG_W * FLAG_H * 4) as usize];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&fill);
    }

    if let Some(pattern) = &coa.pattern {
        let key = texture_key(pattern);
        if let Some(img) = textures.get(&key) {
            let pattern_colors = [
                resolve_color(colors, coa, coa.color1.as_ref())?,
                resolve_color(colors, coa, coa.color2.as_ref()).unwrap_or(fill),
                resolve_color(colors, coa, coa.color3.as_ref()).unwrap_or(fill),
                resolve_color(colors, coa, coa.color4.as_ref()).unwrap_or(fill),
            ];
            recolor_pattern(&mut rgba, img, pattern_colors);
        } else if key != "pattern_solid.tga" {
            return None;
        }
    }

    for emblem in &coa.emblems {
        let key = texture_key(&emblem.texture);
        let Some(img) = textures.get(&key) else {
            continue;
        };
        let emblem_colors = if emblem.colored {
            Some([
                resolve_color(colors, coa, emblem.color1.as_ref().or(coa.color1.as_ref()))?,
                resolve_color(colors, coa, emblem.color2.as_ref().or(coa.color2.as_ref()))
                    .unwrap_or(fill),
                resolve_color(colors, coa, emblem.color3.as_ref().or(coa.color3.as_ref()))
                    .unwrap_or(fill),
            ])
        } else {
            None
        };
        let default_instance = [EmblemInstance::default()];
        let instances = if emblem.instances.is_empty() {
            &default_instance[..]
        } else {
            &emblem.instances
        };
        for instance in instances {
            blit_emblem(&mut rgba, img, emblem_colors, *instance);
        }
    }

    encode_png(FLAG_W, FLAG_H, &rgba)
}

fn texture_key(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_lowercase()
}

#[derive(Debug)]
pub struct RgbaImage {
    w: u32,
    h: u32,
    data: Vec<u8>,
}

fn scale_to_flag(image: &RgbaImage) -> RgbaImage {
    let mut data = Vec::with_capacity((FLAG_W * FLAG_H * 4) as usize);
    for y in 0..FLAG_H {
        for x in 0..FLAG_W {
            let source_x = x * image.w / FLAG_W;
            let source_y = y * image.h / FLAG_H;
            let index = ((source_y * image.w + source_x) * 4) as usize;
            data.extend_from_slice(&image.data[index..index + 4]);
        }
    }
    RgbaImage {
        w: FLAG_W,
        h: FLAG_H,
        data,
    }
}

fn decode_texture(bytes: &[u8]) -> Option<RgbaImage> {
    if bytes.starts_with(b"DDS ") {
        let decoded = icons::coa_dds_to_rgba(bytes)?;
        return Some(RgbaImage {
            w: decoded.width,
            h: decoded.height,
            data: decoded.data,
        });
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

fn recolor_pattern(canvas: &mut [u8], img: &RgbaImage, colors: [[u8; 4]; 4]) {
    for y in 0..FLAG_H {
        for x in 0..FLAG_W {
            let sx = x * img.w / FLAG_W;
            let sy = y * img.h / FLAG_H;
            let si = ((sy * img.w + sx) * 4) as usize;
            let masks = [
                img.data.get(si).copied().unwrap_or(0) as f64 / 255.0,
                img.data.get(si + 1).copied().unwrap_or(0) as f64 / 255.0,
                img.data.get(si + 2).copied().unwrap_or(0) as f64 / 255.0,
                img.data.get(si + 3).copied().unwrap_or(0) as f64 / 255.0,
            ];
            let total = masks.iter().sum::<f64>();
            if total <= f64::EPSILON {
                continue;
            }
            let di = ((y * FLAG_W + x) * 4) as usize;
            for channel in 0..3 {
                canvas[di + channel] = colors
                    .iter()
                    .zip(masks)
                    .map(|(color, mask)| color[channel] as f64 * mask / total)
                    .sum::<f64>()
                    .round() as u8;
            }
            canvas[di + 3] = 255;
        }
    }
}

fn blit_emblem(
    canvas: &mut [u8],
    img: &RgbaImage,
    colors: Option<[[u8; 4]; 3]>,
    instance: EmblemInstance,
) {
    let dw = (FLAG_W as f64 * instance.scale[0].abs()).round() as u32;
    let dh = (FLAG_H as f64 * instance.scale[1].abs()).round() as u32;
    if dw == 0 || dh == 0 {
        return;
    }
    let center_x = (FLAG_W as f64 * instance.position[0]).round() as i64;
    let center_y = (FLAG_H as f64 * instance.position[1]).round() as i64;
    let ox = center_x - dw as i64 / 2;
    let oy = center_y - dh as i64 / 2;
    // Vanilla emblems reach scales of 100 and positions far off the flag, so walk
    // only the part that lands on the canvas rather than clipping per pixel.
    let x_start = ox.max(0);
    let x_end = (ox + dw as i64).min(FLAG_W as i64);
    let y_start = oy.max(0);
    let y_end = (oy + dh as i64).min(FLAG_H as i64);
    if x_start >= x_end || y_start >= y_end {
        return;
    }
    for dy in y_start..y_end {
        let y = (dy - oy) as u64;
        for dx in x_start..x_end {
            let x = (dx - ox) as u64;
            let sx = (x * img.w as u64 / dw as u64) as u32;
            let sy = (y * img.h as u64 / dh as u64) as u32;
            let si = ((sy * img.w + sx) * 4) as usize;
            let sr = img.data.get(si).copied().unwrap_or(0);
            let sg = img.data.get(si + 1).copied().unwrap_or(0);
            let sb = img.data.get(si + 2).copied().unwrap_or(0);
            let sa = img.data.get(si + 3).copied().unwrap_or(0);
            let (rgb, alpha) = if let Some(colors) = colors {
                let masks = [sr as f64 / 255.0, sg as f64 / 255.0, sb as f64 / 255.0];
                let coverage = masks.iter().copied().fold(0.0, f64::max);
                if coverage <= f64::EPSILON {
                    continue;
                }
                let total = masks.iter().sum::<f64>().max(f64::EPSILON);
                let mut rgb = [0u8; 3];
                for channel in 0..3 {
                    rgb[channel] = colors
                        .iter()
                        .zip(masks)
                        .map(|(color, mask)| color[channel] as f64 * mask / total)
                        .sum::<f64>()
                        .round() as u8;
                }
                (rgb, coverage * (sa as f64 / 255.0))
            } else {
                ([sr, sg, sb], sa as f64 / 255.0)
            };
            if alpha <= f64::EPSILON {
                continue;
            }
            let di = (((dy as u32) * FLAG_W + dx as u32) * 4) as usize;
            canvas[di] = ((rgb[0] as f64 * alpha) + canvas[di] as f64 * (1.0 - alpha)) as u8;
            canvas[di + 1] =
                ((rgb[1] as f64 * alpha) + canvas[di + 1] as f64 * (1.0 - alpha)) as u8;
            canvas[di + 2] =
                ((rgb[2] as f64 * alpha) + canvas[di + 2] as f64 * (1.0 - alpha)) as u8;
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

#[derive(Debug)]
struct Entry {
    key: String,
    value: String,
}

/// Parse direct `key = value` entries while preserving duplicate keys. This is
/// intentionally a small object reader, not a general Clausewitz evaluator:
/// scripted expressions remain opaque and therefore cannot become fake flags.
fn entries(object: &str) -> Vec<Entry> {
    let bytes = object.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        skip_space_and_comments(bytes, &mut cursor);
        if cursor >= bytes.len() || bytes[cursor] == b'}' {
            cursor += usize::from(cursor < bytes.len());
            continue;
        }
        let key_start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'{' | b'}')
        {
            cursor += 1;
        }
        let key = object[key_start..cursor].trim();
        skip_inline_space(bytes, &mut cursor);
        if key.is_empty() || cursor >= bytes.len() || bytes[cursor] != b'=' {
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            continue;
        }
        cursor += 1;
        skip_inline_space(bytes, &mut cursor);
        let value_start = cursor;
        let value = if cursor < bytes.len() && bytes[cursor] == b'{' {
            balanced_value(object, &mut cursor)
        } else {
            let prefix_start = cursor;
            if cursor < bytes.len() && bytes[cursor] == b'"' {
                consume_quoted(bytes, &mut cursor);
            } else {
                while cursor < bytes.len()
                    && !bytes[cursor].is_ascii_whitespace()
                    && !matches!(bytes[cursor], b'{' | b'}')
                {
                    cursor += 1;
                }
            }
            let prefix_end = cursor;
            skip_inline_space(bytes, &mut cursor);
            if cursor < bytes.len() && bytes[cursor] == b'{' {
                let block = balanced_value(object, &mut cursor);
                format!("{} {{ {} }}", &object[prefix_start..prefix_end], block)
            } else {
                cursor = value_start;
                let mut quoted = false;
                while cursor < bytes.len() {
                    match bytes[cursor] {
                        b'"' => {
                            quoted = !quoted;
                            cursor += 1;
                        }
                        b'\n' if !quoted => break,
                        b'#' if !quoted => break,
                        b'}' if !quoted => break,
                        _ => cursor += 1,
                    }
                }
                object[value_start..cursor].trim().to_string()
            }
        };
        if !key.starts_with('@') && !value.is_empty() {
            out.push(Entry {
                key: key.to_string(),
                value,
            });
        }
    }
    out
}

fn balanced_value(object: &str, cursor: &mut usize) -> String {
    let bytes = object.as_bytes();
    let open = *cursor;
    let mut depth = 0u32;
    let mut quoted = false;
    while *cursor < bytes.len() {
        match bytes[*cursor] {
            b'"' => quoted = !quoted,
            b'#' if !quoted => {
                while *cursor < bytes.len() && bytes[*cursor] != b'\n' {
                    *cursor += 1;
                }
                continue;
            }
            b'{' if !quoted => depth += 1,
            b'}' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    let value = object[open + 1..*cursor].to_string();
                    *cursor += 1;
                    return value;
                }
            }
            _ => {}
        }
        *cursor += 1;
    }
    String::new()
}

fn skip_space_and_comments(bytes: &[u8], cursor: &mut usize) {
    loop {
        while *cursor < bytes.len() && bytes[*cursor].is_ascii_whitespace() {
            *cursor += 1;
        }
        if *cursor < bytes.len() && bytes[*cursor] == b'#' {
            while *cursor < bytes.len() && bytes[*cursor] != b'\n' {
                *cursor += 1;
            }
        } else {
            break;
        }
    }
}

fn skip_inline_space(bytes: &[u8], cursor: &mut usize) {
    while *cursor < bytes.len() && matches!(bytes[*cursor], b' ' | b'\t' | b'\r') {
        *cursor += 1;
    }
}

fn consume_quoted(bytes: &[u8], cursor: &mut usize) {
    *cursor += 1;
    while *cursor < bytes.len() {
        if bytes[*cursor] == b'"' && bytes.get((*cursor).saturating_sub(1)) != Some(&b'\\') {
            *cursor += 1;
            break;
        }
        *cursor += 1;
    }
}

fn scalar_value(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.starts_with("list") || raw.starts_with('{') {
        return None;
    }
    let value = unquote(raw).trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn unquote(raw: &str) -> &str {
    raw.strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(raw)
}

fn numbers(raw: &str) -> Vec<f64> {
    raw.split(|ch: char| !ch.is_ascii_digit() && !matches!(ch, '.' | '-' | '+' | 'e' | 'E'))
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn pair(raw: &str) -> Option<[f64; 2]> {
    let values = numbers(raw);
    Some([*values.first()?, *values.get(1)?])
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

    #[test]
    fn oversized_emblem_instances_cost_only_the_visible_pixels() {
        let img = RgbaImage {
            w: 4,
            h: 4,
            data: vec![255u8; 4 * 4 * 4],
        };
        let mut canvas = vec![0u8; (FLAG_W * FLAG_H * 4) as usize];
        // Vanilla CoAs really do carry scale 100 at position 11; iterating that
        // whole 6400x4200 rect to fill a 64x42 flag took seconds per emblem.
        let huge = EmblemInstance {
            position: [1.0, 11.0],
            scale: [100.0, 100.0],
        };
        let start = std::time::Instant::now();
        blit_emblem(&mut canvas, &img, None, huge);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "clipping must bound the blit to the canvas"
        );
        assert!(canvas.chunks_exact(4).all(|px| px == [255, 255, 255, 255]));

        let offscreen = EmblemInstance {
            position: [50.0, 50.0],
            scale: [1.0, 1.0],
        };
        let mut untouched = vec![7u8; (FLAG_W * FLAG_H * 4) as usize];
        blit_emblem(&mut untouched, &img, None, offscreen);
        assert!(untouched.iter().all(|byte| *byte == 7));
    }

    #[test]
    fn parses_hsv360_normalized_rgb_and_bare_rgb() {
        let mut colors = BTreeMap::new();
        parse_named_colors(
            br#"
colors = {
    red = hsv360 { 0 100 100 }
    green = hsv { 0.333333 1 1 }
    normalized = rgb { 1 0.5 0 }
    byte = { 32 112 165 }
    hexed = hex { 112233 }
}
"#,
            &mut colors,
        );
        assert_eq!(colors["red"], [255, 0, 0, 255]);
        assert!(colors["green"][1] >= 254);
        assert_eq!(colors["normalized"], [255, 128, 0, 255]);
        assert_eq!(colors["byte"], [32, 112, 165, 255]);
        assert_eq!(colors["hexed"], [0x11, 0x22, 0x33, 255]);
    }

    #[test]
    fn nested_instances_keep_emblem_colors_and_geometry() {
        let mut coats = BTreeMap::new();
        parse_coat_of_arms_file(
            br#"
PRU = {
    pattern = "pattern_solid.tga"
    color1 = "white"
    colored_emblem = {
        texture = "ce_eagle_prussia.dds"
        instance = { scale = { 1.0 0.8 } position = { 0.5 0.4 } }
        instance = { position = { 0.25 0.75 } scale = { 0.2 0.3 } }
        color1 = "black"
        color2 = "yellow"
        color3 = "pearl"
    }
}
"#,
            &mut coats,
        );
        let emblem = &coats["PRU"].emblems[0];
        assert_eq!(emblem.color1.as_deref(), Some("black"));
        assert_eq!(emblem.color2.as_deref(), Some("yellow"));
        assert_eq!(emblem.color3.as_deref(), Some("pearl"));
        assert_eq!(emblem.instances.len(), 2);
        assert_eq!(emblem.instances[1].position, [0.25, 0.75]);
        assert_eq!(emblem.instances[1].scale, [0.2, 0.3]);
    }

    #[test]
    fn resolves_parent_and_unambiguous_template_list_cycle_safely() {
        let mut library = CoaLibrary::default();
        library.colors.insert("blue".into(), [0, 0, 255, 255]);
        library.coats.insert(
            "base".into(),
            CoatOfArms {
                pattern: Some("pattern_solid.tga".into()),
                color1: Some("blue".into()),
                ..CoatOfArms::default()
            },
        );
        library.coats.insert(
            "GBR".into(),
            CoatOfArms {
                parents: vec!["base".into()],
                ..CoatOfArms::default()
            },
        );
        library
            .template_lists
            .insert("single".into(), vec!["base".into()]);
        library.coats.insert(
            "cycle_a".into(),
            CoatOfArms {
                parents: vec!["cycle_b".into()],
                ..CoatOfArms::default()
            },
        );
        library.coats.insert(
            "cycle_b".into(),
            CoatOfArms {
                parents: vec!["cycle_a".into()],
                ..CoatOfArms::default()
            },
        );
        render_library(&mut library, &BTreeMap::new());
        assert!(library.rendered.contains_key("GBR"));
        assert!(library.rendered.contains_key("single"));
        assert!(!library.rendered.contains_key("cycle_a"));
    }

    #[test]
    fn unknown_color_is_unrenderable_not_magenta() {
        let coa = CoatOfArms {
            color1: Some("not_a_color".into()),
            ..CoatOfArms::default()
        };
        assert!(render_coa(&coa, &BTreeMap::new(), &BTreeMap::new()).is_none());
    }
}
