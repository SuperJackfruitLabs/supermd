//! base16 schemes -> SuperMD theme files.
//!
//! `tinted-theming/schemes` publishes 340 palettes in one format: sixteen
//! slots, `base00`-`base07` running background to foreground and
//! `base08`-`base0F` holding red, orange, yellow, green, cyan, blue,
//! magenta and brown, plus a `variant` of `dark` or `light`. Four of
//! those schemes are ones SuperMD already shipped as hand-written
//! themes, so the mapping below is not trusted, it is *measured*: the
//! tests at the bottom convert `nord`, `gruvbox-dark-hard`,
//! `solarized-dark` and `solarized-light` and compare the result with
//! what a human chose, and every difference is a thing the mapping
//! loses.
//!
//! Everything here is pure: text in, text out, no GPUI and no `crate::`.
//! `examples/import_base16.rs` includes this file by path and is the
//! only thing that writes to disk; the binary carries the generated
//! `assets/themes/*.toml` instead, which is why the module is
//! `#[cfg(test)]` in `main.rs`.

// ── colour ──────────────────────────────────────────────────────────

/// An 8-bit sRGB colour. The theme file format speaks hex, the contrast
/// guards speak HSL lightness and WCAG luminance; this is the one type
/// that converts between all three.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Srgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Srgb {
    pub fn parse(s: &str) -> Result<Self, String> {
        let hex = s.trim().trim_start_matches('#');
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("bad hex colour {s:?}"));
        }
        let v = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
        Ok(Srgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 })
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    fn parts(self) -> (f32, f32, f32) {
        (self.r as f32 / 255., self.g as f32 / 255., self.b as f32 / 255.)
    }

    fn of(r: f32, g: f32, b: f32) -> Self {
        let q = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        Srgb { r: q(r), g: q(g), b: q(b) }
    }

    /// HSL, computed exactly the way `gpui::Hsla` does it -- the contrast
    /// guards compare lightnesses, so a different formula here would be
    /// measuring a different colour than the one that ships.
    pub fn hsl(self) -> (f32, f32, f32) {
        let (r, g, b) = self.parts();
        let max = r.max(g.max(b));
        let min = r.min(g.min(b));
        let d = max - min;
        let l = (max + min) / 2.;
        let s = if l == 0. || l == 1. {
            0.
        } else if l < 0.5 {
            d / (2. * l)
        } else {
            d / (2. - 2. * l)
        };
        let h = if d == 0. {
            0.
        } else if max == r {
            (((g - b) / d).rem_euclid(6.)) / 6.
        } else if max == g {
            ((b - r) / d + 2.) / 6.
        } else {
            ((r - g) / d + 4.) / 6.
        };
        (h, s, l)
    }

    pub fn from_hsl(h: f32, s: f32, l: f32) -> Self {
        let l = l.clamp(0., 1.);
        let f = |n: f32| {
            let k = (n + h * 12.).rem_euclid(12.);
            let a = s.clamp(0., 1.) * l.min(1. - l);
            l - a * (-1f32).max((k - 3.).min((9. - k).min(1.)))
        };
        Srgb::of(f(0.), f(8.), f(4.))
    }

    pub fn lightness(self) -> f32 {
        self.hsl().2
    }

    fn luminance(self) -> f32 {
        let (r, g, b) = self.parts();
        let f = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    }
}

/// WCAG contrast ratio. Same formula as `Theme::contrast`, over opaque
/// colours: everything this module emits is opaque except `shadow`,
/// whose alpha is a strength rather than a colour.
pub fn contrast(a: Srgb, b: Srgb) -> f32 {
    let (x, y) = (a.luminance(), b.luminance());
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// Linear interpolation in sRGB.
pub fn mix(a: Srgb, b: Srgb, t: f32) -> Srgb {
    let t = t.clamp(0., 1.);
    let (ar, ag, ab) = a.parts();
    let (br, bg, bb) = b.parts();
    Srgb::of(ar + (br - ar) * t, ag + (bg - ag) * t, ab + (bb - ab) * t)
}

const WHITE: Srgb = Srgb { r: 255, g: 255, b: 255 };
const BLACK: Srgb = Srgb { r: 0, g: 0, b: 0 };

/// A step of `dl` in lightness, taken as a mix toward white or black.
///
/// Not an HSL lightness assignment, which is the obvious way to write
/// this and the wrong one: a saturated ground (solarized-dark's
/// `#002b36` is at saturation 1.0) raised in HSL gets *more* saturated
/// as it lightens, and turns a dark teal desk into a vivid one. Mixing
/// toward white desaturates on the way, which is what the hand-tuned
/// themes did -- `nord`'s desk is `page_bg` mixed 16% toward black, to
/// the byte.
pub fn step(c: Srgb, dl: f32, up: bool) -> Srgb {
    let l = c.lightness();
    if up {
        let t = if l >= 1. { 0. } else { (dl / (1. - l)).min(1.) };
        mix(c, WHITE, t)
    } else {
        let t = if l <= 0. { 0. } else { (dl / l).min(1.) };
        mix(c, BLACK, t)
    }
}

/// Distance between two hues on the circle, in turns (0..=0.5).
fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(1.);
    d.min(1. - d)
}

// ── the scheme file ─────────────────────────────────────────────────

pub const SLOTS: [&str; 16] = [
    "base00", "base01", "base02", "base03", "base04", "base05", "base06", "base07", "base08",
    "base09", "base0A", "base0B", "base0C", "base0D", "base0E", "base0F",
];

#[derive(Clone, Debug)]
pub struct Scheme {
    pub slug: String,
    pub name: String,
    pub author: String,
    pub dark: bool,
    pub palette: [Srgb; 16],
}

impl Scheme {
    /// Reads the subset of YAML these files actually use: three quoted
    /// scalars and sixteen `baseXX: "#rrggbb"` entries, with trailing
    /// `#` comments (several schemes annotate their slots) and either
    /// quoting style. A real YAML parser would be a dependency for one
    /// file shape that upstream's own spec pins.
    pub fn parse(slug: &str, yaml: &str) -> Result<Self, String> {
        let mut name = String::new();
        let mut author = String::new();
        let mut variant = String::new();
        let mut palette = [None; 16];
        for raw in yaml.lines() {
            let line = raw.trim_end();
            let indented = line.starts_with(' ');
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, rest)) = line.split_once(':') else { continue };
            let key = key.trim();
            let value = unquote(rest);
            if value.is_empty() {
                continue;
            }
            match key {
                "name" if !indented => name = value.to_string(),
                "author" if !indented => author = value.to_string(),
                "variant" if !indented => variant = value.to_string(),
                _ => {
                    if let Some(i) = SLOTS.iter().position(|s| *s == key) {
                        palette[i] = Some(Srgb::parse(value)?);
                    }
                }
            }
        }
        if name.is_empty() {
            return Err(format!("{slug}: no name"));
        }
        let dark = match variant.as_str() {
            "dark" => true,
            "light" => false,
            other => return Err(format!("{slug}: bad variant {other:?}")),
        };
        let mut colours = [Srgb { r: 0, g: 0, b: 0 }; 16];
        for (i, slot) in palette.iter().enumerate() {
            colours[i] = slot.ok_or_else(|| format!("{slug}: missing {}", SLOTS[i]))?;
        }
        Ok(Scheme { slug: slug.to_string(), name, author, dark, palette: colours })
    }

    fn slot(&self, name: &str) -> Srgb {
        let i = SLOTS.iter().position(|s| *s == name).expect("known slot");
        self.palette[i]
    }
}

/// Strips an inline `# comment` and surrounding quotes from a YAML
/// scalar. Hex colours start with `#`, so the comment strip only looks
/// for a `#` that follows whitespace.
fn unquote(rest: &str) -> &str {
    let mut value = rest.trim();
    if let Some(i) = value.find(" #") {
        value = value[..i].trim_end();
    }
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value = &value[1..value.len() - 1];
    }
    value.trim()
}

// ── the mapping ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub bg: Srgb,
    pub page_bg: Srgb,
    pub panel_bg: Srgb,
    pub floating_bg: Srgb,
    pub hover_bg: Srgb,
    pub selected_bg: Srgb,
    pub code_bg: Srgb,
    pub border: Srgb,
    pub fg: Srgb,
    pub fg_strong: Srgb,
    pub fg_muted: Srgb,
    pub code_fg: Srgb,
    pub accent: Srgb,
    pub link: Srgb,
    pub find_match_bg: Srgb,
    pub find_active_bg: Srgb,
    pub shadow: Srgb,
    pub shadow_alpha: u8,
    pub diff_added_bg: Srgb,
    pub diff_added_fg: Srgb,
    pub diff_deleted_bg: Srgb,
    pub diff_deleted_fg: Srgb,
    pub syntax: [Srgb; 10],
}

pub const SYNTAX_KEYS: [&str; 10] = [
    "keyword", "function", "type", "string", "comment", "constant", "property", "operator", "tag",
    "attribute",
];

/// How far a surface may sit from the page, in HSL lightness, and which
/// palette slot supplies it when it lands inside that band.
///
/// The bands are not invented: they are the spread of the same tokens
/// across the six themes SuperMD hand-tuned, so a scheme whose own slot
/// falls inside one is used as published and only a slot outside it is
/// replaced by our own step. `nord`'s `base02` is +0.100 from its page
/// and ships verbatim as the border; `solarized-dark`'s is +0.296, four
/// times the widest rung any hand-tuned theme uses, and gets clamped.
struct Ladder {
    /// The desk under the page, and the chrome panel under that.
    desk: f32,
    panel: f32,
    /// The overlay surface, and the hovered row painted on it.
    floating: f32,
    hover: f32,
    /// `base02` -> selection, `base01` -> code fence, `base02` -> border.
    selected: (f32, f32),
    code: (f32, f32),
    border: (f32, f32),
}

const DARK: Ladder = Ladder {
    desk: 0.035,
    panel: 0.052,
    floating: 0.015,
    hover: 0.038,
    selected: (0.060, 0.090),
    code: (0.052, 0.090),
    border: (0.060, 0.105),
};

/// A light page is at or near white, where equal steps in lightness buy
/// much less contrast than they do at the dark end, so every rung is
/// wider. `floating` is unused: an overlay takes the page itself,
/// because there is no brighter step to take (`Theme::derive_floating_bg`).
const LIGHT: Ladder = Ladder {
    desk: 0.035,
    panel: 0.058,
    floating: 0.0,
    hover: 0.095,
    selected: (0.118, 0.155),
    code: (0.045, 0.100),
    border: (0.110, 0.165),
};

/// The mapping, as one pure function.
///
/// The shape of it: `base00` is the page -- the sheet being read, and
/// the colour the scheme is recognised by -- and every other surface is
/// a step from it, taken in the scheme's own ground rather than in
/// grey. `base01` and `base02` are used where the scheme puts them
/// inside our band and replaced by our own step where it does not.
/// Ink is `base05`, the published foreground, with `fg_muted` searched
/// along `base03` -> `base05` because "secondary chrome text" is not one
/// of base16's sixteen roles and `base03` -- the comments slot -- is dim
/// by design.
pub fn map(scheme: &Scheme) -> Tokens {
    let b = |n: &str| scheme.slot(n);
    let dark = scheme.dark;
    let l = if dark { DARK } else { LIGHT };
    // The page is the scheme's own background -- the sheet being read,
    // and the colour a palette is recognised by -- except where that
    // ground is so near black that nothing fits underneath it. Then the
    // scheme's colour becomes the desk and the page takes the step up
    // instead: the same move solarized-light's hand-written theme makes
    // in the mirror case, where base00 could not carry the body ink and
    // the page went brighter than the palette's own background.
    let ground = b("base00");
    let airless = ground.lightness() < 0.030;
    let page = if airless { step(ground, l.desk, true) } else { ground };
    let down = |dl: f32| {
        if airless {
            step(ground, (ground.lightness() * 0.5).max(0.004), false)
        } else {
            step(page, dl, false)
        }
    };

    let rung = |slot: Srgb, (lo, hi): (f32, f32)| -> Srgb {
        let d = if dark { slot.lightness() - page.lightness() } else { page.lightness() - slot.lightness() };
        if (lo..=hi).contains(&d) {
            slot
        } else {
            step(page, if d < lo { lo } else { hi }, dark)
        }
    };

    let bg = if airless { ground } else { down(l.desk) };
    let panel_bg = down(l.panel);
    let hover_bg = step(page, l.hover, dark);
    let floating_bg = if dark { step(page, l.floating, true) } else { page };
    let selected_bg = rung(b("base02"), l.selected);
    let code_bg = rung(b("base01"), l.code);
    let border = rung(b("base02"), l.border);

    let fg = b("base05");
    // base06 and base07 are "not often used" in base16 and several
    // schemes park something odd there -- nord's base07 is a teal, and
    // zenburn's base06 is darker than its base05. Whichever reads
    // strongest on the page is the heading ink; if that is base05
    // itself, step it away from the page instead.
    let fg_strong = [b("base05"), b("base06"), b("base07")]
        .into_iter()
        .max_by(|a, c| {
            contrast(*a, page).partial_cmp(&contrast(*c, page)).expect("finite contrast")
        })
        .expect("three candidates");
    let fg_strong = if fg_strong == fg { step(fg, 0.10, dark) } else { fg_strong };

    let surfaces = [bg, page, panel_bg, floating_bg, hover_bg, selected_bg];
    let worst = |ink: Srgb, over: &[Srgb]| {
        over.iter().map(|s| contrast(ink, *s)).fold(f32::INFINITY, f32::min)
    };
    // Muted ink is written on all six surfaces plus the code fence, and
    // `selected_bg` -- the far end of the ramp -- is what decides. Walk
    // from the comments slot toward the foreground and stop at the first
    // step that clears the floor, so the hint stays as dim as the
    // palette allows while staying readable. Capped at 80% of the way:
    // ink that reads as body text is not a muted token, it is a second
    // body token, and a scheme that cannot get there gets an exception
    // in `theme.rs` rather than a louder hint.
    let mut muted_surfaces = surfaces.to_vec();
    muted_surfaces.push(code_bg);
    let mut fg_muted = b("base03");
    for i in 0..=16 {
        let t = i as f32 * 0.05;
        fg_muted = mix(b("base03"), b("base05"), t);
        if worst(fg_muted, &muted_surfaces) >= 3.05 || t >= 0.80 {
            break;
        }
    }

    // Code text is the smallest text in the app and the fence is a
    // surface the theme did not choose, so this one is pushed until it
    // clears rather than exempted. solarized-light's hand-written file
    // does the same thing by hand, four units at a time.
    let mut code_fg = fg;
    for _ in 0..45 {
        if contrast(code_fg, code_bg) >= 4.55 {
            break;
        }
        code_fg = step(code_fg, 0.02, dark);
    }

    // The comments slot is dim by design and several schemes take that
    // further than any theme SuperMD ships by hand: catppuccin's base03
    // is 1.4:1 on its own fence, where the dimmest hand-tuned comment
    // (nord's) is 1.96:1. Walk the same base03 -> base05 line the muted
    // hint walks, far enough to read at all. Code comments are not body
    // text and the palettes mean them to recede, so the floor is that
    // dimmest shipped value rather than a text ratio.
    let mut comment = b("base03");
    for i in 0..=16 {
        let t = i as f32 * 0.05;
        comment = mix(b("base03"), b("base05"), t);
        if contrast(comment, code_bg) >= 2.0 || t >= 0.80 {
            break;
        }
    }

    // base16's search highlight is base0A. Its hue and saturation carry
    // the palette; its lightness has to sit a fixed distance from the
    // page or a light scheme's yellow washes out and a dark one glares.
    let (ha, sa, _) = b("base0A").hsl();
    let sa = sa.clamp(0.35, 0.75);
    let lp = page.lightness();
    let find_match_bg = Srgb::from_hsl(ha, sa, if dark { lp + 0.085 } else { lp - 0.170 });
    let find_active_bg = Srgb::from_hsl(ha, sa, if dark { lp + 0.190 } else { lp - 0.350 });

    // The ground's own hue, deepened, so the shadow darkens the desk
    // rather than draining it -- a neutral grey shadow greys a warm
    // theme. A dark page has little room below it, so the floor is half
    // its lightness; a light page has all the room in the world and
    // takes a fixed deep tint, saturated a little to survive the alpha.
    let (hp, sp, _) = page.hsl();
    let shadow = if dark {
        Srgb::from_hsl(hp, sp, (lp * 0.5).max(lp - 0.115))
    } else {
        Srgb::from_hsl(hp, (sp * 1.5 + 0.08).min(0.35), 0.20)
    };

    let added = nearest_hue(scheme, 120. / 360., "base0B");
    let removed = nearest_hue(scheme, 0., "base08");

    Tokens {
        bg,
        page_bg: page,
        panel_bg,
        floating_bg,
        hover_bg,
        selected_bg,
        code_bg,
        border,
        fg,
        fg_strong,
        fg_muted,
        code_fg,
        // base0D, not base08. The three hand-written themes all made
        // their accent the scheme's red, but base08's published meaning
        // is "variables", not "red", and the registry takes it
        // literally: all three tokyo-night schemes put their foreground
        // there, which would make the primary button near-white.
        accent: b("base0D"),
        link: b("base0D"),
        find_match_bg,
        find_active_bg,
        shadow,
        // 0.34 dark / 0.18 light, the two hand-tuned strengths, both
        // well under the 0.5 the loader clamps at.
        shadow_alpha: if dark { 0x57 } else { 0x2e },
        diff_added_bg: mix(page, added, 0.18),
        diff_added_fg: added,
        diff_deleted_bg: mix(page, removed, 0.18),
        diff_deleted_fg: removed,
        syntax: [
            b("base0E"), // keyword: keywords, storage, selector
            b("base0D"), // function: functions, methods, headings
            b("base0A"), // type: classes, types
            b("base0B"), // string: strings, inherited class
            comment,     // comment: base03, lifted only where it cannot be read
            b("base09"), // constant: integers, booleans, constants
            b("base08"), // property: variables, markup link text
            b("base05"), // operator: delimiters and operators
            b("base08"), // tag: XML/HTML tags
            b("base0A"), // attribute: alongside types, as the fixtures do
        ],
    }
}

/// base16 names `base0B` green and `base08` red, and the diff view reads
/// those two as *added* and *removed* rather than as decoration. Not
/// every published scheme obeys the naming: github puts a dark blue in
/// `base0B` and all three tokyo-nights put their foreground in `base08`.
/// So the named slot is kept unless its hue is more than a quarter turn
/// from the target, and only then is the nearest usable accent taken --
/// which finds github's own green and tokyo-night's own red.
fn nearest_hue(scheme: &Scheme, target: f32, slot: &str) -> Srgb {
    let named = scheme.slot(slot);
    if hue_distance(named.hsl().0, target) <= 0.25 {
        return named;
    }
    let mut best = named;
    let mut best_d = 1.0;
    for name in &SLOTS[8..] {
        let c = scheme.slot(name);
        let (h, s, l) = c.hsl();
        if s < 0.25 || !(0.12..=0.92).contains(&l) {
            continue;
        }
        let d = hue_distance(h, target);
        if d < best_d {
            best = c;
            best_d = d;
        }
    }
    best
}

// ── the theme file ──────────────────────────────────────────────────

/// The schemes converted into shipped themes. The other four vendored
/// under `assets/base16/` are fixtures for the tests, not themes:
/// SuperMD already ships hand-tuned files for those palettes and the
/// converter is measured against them.
pub const CURATED: [&str; 20] = [
    "ayu-dark",
    "ayu-light",
    "catppuccin-frappe",
    "catppuccin-latte",
    "catppuccin-macchiato",
    "catppuccin-mocha",
    "dracula",
    "everforest",
    "github",
    "kanagawa",
    "monokai",
    "one-light",
    "onedark",
    "rose-pine",
    "rose-pine-dawn",
    "rose-pine-moon",
    "tokyo-night-dark",
    "tokyo-night-light",
    "tokyo-night-storm",
    "zenburn",
];

/// Which base16 slot a colour came from, or `derived` when the mapping
/// stepped it from the ground. Computed by comparison rather than
/// recorded as the mapping runs, so the annotation cannot disagree with
/// the value beside it.
fn provenance(scheme: &Scheme, c: Srgb) -> &'static str {
    match scheme.palette.iter().position(|p| *p == c) {
        Some(i) => SLOTS[i],
        None => "derived",
    }
}

/// One theme file, ready to write to `assets/themes/<slug>.toml`.
pub fn render_toml(scheme: &Scheme) -> String {
    let t = map(scheme);
    let mut out = String::new();
    out.push_str(&format!(
        "# Generated from base16 by `cargo run --example import_base16`.\n\
         # Do not edit by hand: regenerate instead, and let the contrast\n\
         # guards in src/theme.rs decide whether the result ships.\n\
         #\n\
         # Scheme: {} by {}\n\
         # Source: tinted-theming/schemes (MIT), vendored at\n\
         # assets/base16/{}.yaml -- see assets/base16/README.md.\n\
         #\n\
         # Each colour is tagged with the slot it came from, or `derived`\n\
         # where the mapping took its own step from the ground.\n\n",
        scheme.name, scheme.author, scheme.slug
    ));
    out.push_str(&format!("name = \"{}\"\n", scheme.name));
    out.push_str(&format!(
        "appearance = \"{}\"\n\n[colors]\n",
        if scheme.dark { "dark" } else { "light" }
    ));

    fn line(out: &mut String, key: &str, value: String, note: &str) {
        let assignment = format!("{key} = \"{value}\"");
        out.push_str(&format!("{assignment:<30}# {note}\n"));
    }
    for (key, c) in [
        ("bg", t.bg),
        ("page_bg", t.page_bg),
        ("panel_bg", t.panel_bg),
        ("floating_bg", t.floating_bg),
        ("hover_bg", t.hover_bg),
        ("selected_bg", t.selected_bg),
        ("border", t.border),
        ("code_bg", t.code_bg),
        ("code_fg", t.code_fg),
        ("fg", t.fg),
        ("fg_strong", t.fg_strong),
        ("fg_muted", t.fg_muted),
        ("accent", t.accent),
        ("link", t.link),
        ("find_match_bg", t.find_match_bg),
        ("find_active_bg", t.find_active_bg),
    ] {
        line(&mut out, key, c.hex(), provenance(scheme, c));
    }
    line(
        &mut out,
        "shadow",
        format!("{}{:02x}", t.shadow.hex(), t.shadow_alpha),
        "derived, translucent",
    );
    for (key, c) in [
        ("diff_added_bg", t.diff_added_bg),
        ("diff_added_fg", t.diff_added_fg),
        ("diff_deleted_bg", t.diff_deleted_bg),
        ("diff_deleted_fg", t.diff_deleted_fg),
    ] {
        line(&mut out, key, c.hex(), provenance(scheme, c));
    }
    out.push_str("\n[syntax]\n");
    for (key, c) in SYNTAX_KEYS.iter().zip(t.syntax) {
        line(&mut out, key, c.hex(), provenance(scheme, c));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four schemes SuperMD already ships as hand-written themes,
    /// paired with the file a human wrote. They are vendored as fixtures
    /// only: no theme is generated from them, and the hand-written files
    /// are never overwritten.
    const FIXTURES: [(&str, &str, &str); 4] = [
        (
            "nord",
            include_str!("../assets/base16/nord.yaml"),
            include_str!("../assets/themes/nord.toml"),
        ),
        (
            "gruvbox-dark-hard",
            include_str!("../assets/base16/gruvbox-dark-hard.yaml"),
            include_str!("../assets/themes/gruvbox-dark.toml"),
        ),
        (
            "solarized-dark",
            include_str!("../assets/base16/solarized-dark.yaml"),
            include_str!("../assets/themes/solarized-dark.toml"),
        ),
        (
            "solarized-light",
            include_str!("../assets/base16/solarized-light.yaml"),
            include_str!("../assets/themes/solarized-light.toml"),
        ),
    ];

    /// `key = "#rrggbb"` out of a theme file, comments and sections
    /// ignored. Enough to read a fixture back for comparison.
    fn theme_colours(toml_src: &str) -> Vec<(String, Srgb)> {
        let mut out = Vec::new();
        for line in toml_src.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim().trim_matches('"');
            // `shadow` carries an alpha byte the fixtures compare on hue only.
            let value = if value.len() == 9 { &value[..7] } else { value };
            if let Ok(c) = Srgb::parse(value) {
                out.push((key.trim().to_string(), c));
            }
        }
        out
    }

    /// The converter against the only reference that is not its own
    /// opinion: four themes a human tuned by hand, from the same four
    /// palettes. Surfaces are held to a tight tolerance because the
    /// ladder is the part of the mapping that can be got objectively
    /// wrong; the inks and the accents are reported rather than
    /// asserted, since which slot a community calls "the foreground" is
    /// a judgement the palette does not record.
    #[test]
    fn the_converter_reproduces_the_hand_written_surfaces() {
        let mut report = String::new();
        let mut worst: Vec<(String, f32)> = Vec::new();
        for (slug, yaml, hand) in FIXTURES {
            let scheme = Scheme::parse(slug, yaml).expect("fixture parses");
            let t = map(&scheme);
            let hand = theme_colours(hand);
            let ours: Vec<(&str, Srgb)> = vec![
                ("bg", t.bg),
                ("page_bg", t.page_bg),
                ("panel_bg", t.panel_bg),
                ("floating_bg", t.floating_bg),
                ("hover_bg", t.hover_bg),
                ("selected_bg", t.selected_bg),
                ("code_bg", t.code_bg),
                ("border", t.border),
            ];
            for (key, mine) in ours {
                let Some((_, theirs)) = hand.iter().find(|(k, _)| k == key) else { continue };
                let dl = mine.lightness() - theirs.lightness();
                report.push_str(&format!(
                    "{slug}.{key}: ours {} hand {} dL {dl:+.3}\n",
                    mine.hex(),
                    theirs.hex()
                ));
                worst.push((format!("{slug}.{key}"), dl.abs()));
            }
        }
        let over: Vec<&(String, f32)> = worst.iter().filter(|(_, d)| *d > 0.055).collect();
        assert!(
            over.is_empty(),
            "surfaces drifted from the hand-written themes: {over:?}\n{report}"
        );
    }

    /// The scheme yaml and the theme toml for every converted theme,
    /// paired at compile time so the staleness check cannot be fooled by
    /// a file that was never read.
    macro_rules! converted {
        ($($slug:literal),+ $(,)?) => {
            const CONVERTED: &[(&str, &str, &str)] = &[
                $((
                    $slug,
                    include_str!(concat!("../assets/base16/", $slug, ".yaml")),
                    include_str!(concat!("../assets/themes/", $slug, ".toml")),
                )),+
            ];
        };
    }
    converted!(
        "ayu-dark",
        "ayu-light",
        "catppuccin-frappe",
        "catppuccin-latte",
        "catppuccin-macchiato",
        "catppuccin-mocha",
        "dracula",
        "everforest",
        "github",
        "kanagawa",
        "monokai",
        "one-light",
        "onedark",
        "rose-pine",
        "rose-pine-dawn",
        "rose-pine-moon",
        "tokyo-night-dark",
        "tokyo-night-light",
        "tokyo-night-storm",
        "zenburn",
    );

    /// The committed theme files are the generator's output, and nothing
    /// at runtime re-runs the generator -- so without this, editing the
    /// mapping and forgetting to regenerate would ship the old colours
    /// while the tests measured the new ones. Same arrangement, and same
    /// hazard, as `docs/site/shortcuts.md`.
    #[test]
    fn every_committed_theme_is_its_scheme_regenerated() {
        for (slug, yaml, committed) in CONVERTED {
            let scheme = Scheme::parse(slug, yaml).expect("scheme parses");
            assert_eq!(
                render_toml(&scheme),
                *committed,
                "{slug}.toml is stale: run `cargo run --example import_base16`"
            );
        }
    }

    /// The generator walks `CURATED`; the staleness check walks its own
    /// compile-time table. A scheme added to one and not the other would
    /// either ship unchecked or be checked and never written.
    #[test]
    fn the_curated_list_and_the_checked_list_are_the_same() {
        let checked: Vec<&str> = CONVERTED.iter().map(|(s, _, _)| *s).collect();
        assert_eq!(checked, CURATED.to_vec());
        let mut sorted = CURATED.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, CURATED.to_vec(), "keep CURATED sorted");
    }

    /// Code comments are the one syntax colour a palette routinely makes
    /// unreadable: several schemes' `base03` sits at 1.4:1 on their own
    /// fence. The mapping lifts it just far enough, and this is the
    /// floor it lifts to -- the dimmest comment any hand-tuned SuperMD
    /// theme ships (nord's `#616e88`, 1.96:1).
    #[test]
    fn converted_comments_can_be_read_on_their_own_fence() {
        for (slug, yaml, _) in CONVERTED {
            let scheme = Scheme::parse(slug, yaml).expect("scheme parses");
            let t = map(&scheme);
            let ratio = contrast(t.syntax[4], t.code_bg);
            assert!(ratio >= 2.0, "{slug}: comments are {ratio:.2}:1 on the code fence");
        }
    }

    /// The page is the scheme's own `base00`, and that is the one thing
    /// about this mapping no contrast guard can check: swap `base00` for
    /// `base01` and the whole ladder simply rebuilds itself around the
    /// new ground, self-consistent and a different theme. Measured when
    /// this test was written -- the swap left every structural guard
    /// green and was caught only by the floor of ayu-light's exception
    /// band, at 4.09:1 against 4.1. So the identity is asserted
    /// directly: a palette is recognised by its background, and a
    /// "Dracula" whose page is not `#282a36` is not Dracula.
    ///
    /// The one exception is a ground with nothing underneath it, where
    /// the scheme's colour becomes the desk instead -- none of the
    /// twenty is that dark, so this also pins that the escape hatch
    /// stays shut.
    #[test]
    fn the_page_is_the_schemes_own_background() {
        for (slug, yaml, _) in CONVERTED {
            let scheme = Scheme::parse(slug, yaml).expect("scheme parses");
            let t = map(&scheme);
            assert_eq!(
                t.page_bg,
                scheme.slot("base00"),
                "{slug}: the page is not the scheme's own background"
            );
            assert_eq!(t.fg, scheme.slot("base05"), "{slug}: the body ink is not base05");
        }
    }

    /// Every converted theme's `fg_muted` is the dimmest step along
    /// `base03` -> `base05` that clears the floor, so it must stay
    /// visibly dimmer than the body ink -- otherwise the hint is a
    /// second body token, and the search has gone too far.
    #[test]
    fn muted_ink_stays_dimmer_than_body_ink() {
        for (slug, yaml, _) in CONVERTED {
            let scheme = Scheme::parse(slug, yaml).expect("scheme parses");
            let t = map(&scheme);
            let (body, muted) = (contrast(t.fg, t.page_bg), contrast(t.fg_muted, t.page_bg));
            assert!(
                muted < body * 0.95,
                "{slug}: muted {muted:.2}:1 is not dimmer than body {body:.2}:1"
            );
        }
    }

    #[test]
    fn scheme_parsing_reads_comments_quotes_and_rejects_damage() {
        let (_, yaml, _) = CONVERTED.iter().find(|(s, _, _)| *s == "dracula").expect("dracula");
        // dracula's file is the awkward one: a description, block
        // comments, and a `# Red` note after every colour.
        let scheme = Scheme::parse("dracula", yaml).expect("parses");
        assert_eq!(scheme.name, "Dracula");
        assert!(scheme.dark);
        assert_eq!(scheme.palette[0], Srgb::parse("#282a36").unwrap());
        assert_eq!(scheme.palette[15], Srgb::parse("#993333").unwrap());

        assert!(Scheme::parse("x", "name: \"X\"\nvariant: \"dark\"\n").is_err(), "no palette");
        assert!(
            Scheme::parse("x", &yaml.replace("variant: \"dark\"", "variant: \"teal\"")).is_err(),
            "bad variant"
        );
        assert!(Scheme::parse("x", &yaml.replacen("name:", "nome:", 1)).is_err(), "no name");
        assert!(
            Scheme::parse("x", &yaml.replace("\"#282a36\"", "\"#28\"")).is_err(),
            "bad hex colour"
        );
    }

    /// A ground with nothing underneath it. base16 allows `base00` to be
    /// pure black, and a desk darker than black does not exist -- so the
    /// scheme's own colour becomes the desk and the page steps up,
    /// rather than the two collapsing into one surface.
    #[test]
    fn an_airless_ground_becomes_the_desk_and_the_page_steps_up() {
        let mut yaml = String::from("name: \"Void\"\nauthor: \"t\"\nvariant: \"dark\"\npalette:\n");
        for (i, slot) in SLOTS.iter().enumerate() {
            let shade = if i < 8 { i * 4 } else { 128 + i * 4 };
            yaml.push_str(&format!("  {slot}: \"#{shade:02x}{shade:02x}{shade:02x}\"\n"));
        }
        let scheme = Scheme::parse("void", &yaml).expect("parses");
        assert_eq!(scheme.palette[0], Srgb { r: 0, g: 0, b: 0 }, "base00 is pure black");
        let t = map(&scheme);
        assert_eq!(t.bg, scheme.palette[0], "the scheme's ground becomes the desk");
        assert!(t.page_bg.lightness() > t.bg.lightness(), "the page steps up off it");
        let delta = t.page_bg.lightness() - t.bg.lightness();
        assert!((0.012..=0.075).contains(&delta), "page/ground delta {delta} is not one step");
        assert!(t.panel_bg.lightness() <= t.bg.lightness(), "the panel stays at or under the desk");
        assert!(t.floating_bg.lightness() > t.page_bg.lightness(), "overlays still lift");
    }

    /// The step has to be taken as a mix toward white, not as an HSL
    /// lightness assignment: a fully saturated ground raised in HSL gets
    /// more saturated as it lightens. solarized-dark's `#002b36` is the
    /// real case -- saturation 1.0 -- and its hand-written desk is its
    /// page mixed toward black, to the byte.
    #[test]
    fn steps_desaturate_toward_white_instead_of_saturating() {
        let teal = Srgb::parse("#002b36").unwrap();
        assert_eq!(teal.hsl().1, 1.0, "the fixture case is fully saturated");
        let lifted = step(teal, 0.09, true);
        assert!(lifted.hsl().1 < teal.hsl().1, "a step up must not saturate");
        assert!((lifted.lightness() - teal.lightness() - 0.09).abs() < 0.006, "and it is a step");
        let hsl_lift = Srgb::from_hsl(teal.hsl().0, teal.hsl().1, teal.lightness() + 0.09);
        assert_ne!(lifted, hsl_lift, "an HSL lift is the wrong colour, not a rounding away");
        // Down, and the clamps at both ends.
        assert!(step(teal, 0.09, false).lightness() < teal.lightness());
        assert_eq!(step(Srgb::parse("#ffffff").unwrap(), 0.1, true).hex(), "#ffffff");
        assert_eq!(step(Srgb::parse("#000000").unwrap(), 0.1, false).hex(), "#000000");
    }

    #[test]
    fn colour_conversions_round_trip_and_reject_garbage() {
        for hex in ["#000000", "#ffffff", "#2e3440", "#f38ba8", "#0db9d7"] {
            let c = Srgb::parse(hex).unwrap();
            assert_eq!(c.hex(), hex);
            let (h, s, l) = c.hsl();
            let back = Srgb::from_hsl(h, s, l);
            assert!(
                (back.r as i32 - c.r as i32).abs() <= 1
                    && (back.g as i32 - c.g as i32).abs() <= 1
                    && (back.b as i32 - c.b as i32).abs() <= 1,
                "{hex} round-tripped to {}",
                back.hex()
            );
        }
        assert_eq!(Srgb::parse("2e3440").unwrap().hex(), "#2e3440", "the hash is optional");
        assert!(Srgb::parse("#2e344").is_err());
        assert!(Srgb::parse("#zzzzzz").is_err());
        let (black, white) = (Srgb::parse("#000000").unwrap(), Srgb::parse("#ffffff").unwrap());
        assert!((contrast(black, white) - 21.0).abs() < 0.01);
        assert_eq!(contrast(black, white), contrast(white, black));
        assert_eq!(mix(black, white, 0.0), black);
        assert_eq!(mix(black, white, 1.0), white);
    }

    /// base16 names `base0B` green and `base08` red and the diff view
    /// reads them as added and removed, but the registry does not always
    /// obey: github puts a dark blue in `base0B`, all three tokyo-nights
    /// put their foreground in `base08`. The substitution finds each
    /// scheme's real green and red, and leaves every obedient scheme's
    /// slots alone.
    #[test]
    fn diff_colours_fall_back_to_the_palettes_real_green_and_red() {
        let mut substituted = Vec::new();
        for (slug, yaml, _) in CONVERTED {
            let scheme = Scheme::parse(slug, yaml).expect("scheme parses");
            let t = map(&scheme);
            if t.diff_added_fg != scheme.slot("base0B") {
                substituted.push(format!("{slug} added {}", t.diff_added_fg.hex()));
            }
            if t.diff_deleted_fg != scheme.slot("base08") {
                substituted.push(format!("{slug} removed {}", t.diff_deleted_fg.hex()));
            }
            // Whatever was chosen, it has to be the right end of the wheel.
            assert!(
                hue_distance(t.diff_added_fg.hsl().0, 120. / 360.) <= 0.25,
                "{slug}: added is not green"
            );
            assert!(
                hue_distance(t.diff_deleted_fg.hsl().0, 0.) <= 0.25,
                "{slug}: removed is not red"
            );
        }
        assert_eq!(
            substituted,
            [
                "github added #116329",
                "tokyo-night-dark removed #f7768e",
                "tokyo-night-light removed #8c4351",
                "tokyo-night-storm removed #f7768e",
            ],
            "the set of schemes that disobey base16's naming changed"
        );
    }

    /// Every colour in a generated file is either a palette slot,
    /// annotated with which, or `derived` -- and the annotation is
    /// computed by comparison, so it cannot drift from the value beside
    /// it. This pins how much of a theme is the scheme's own: the first
    /// version of the mapping derived nearly everything, which looked
    /// faithful in prose and was not.
    #[test]
    fn most_of_a_converted_theme_comes_straight_from_the_palette() {
        for (slug, yaml, committed) in CONVERTED {
            let scheme = Scheme::parse(slug, yaml).expect("scheme parses");
            let from_palette = committed.lines().filter(|l| l.contains("# base0")).count();
            assert!(
                from_palette >= 14,
                "{slug}: only {from_palette} colours come from the palette"
            );
            // And the annotation is honest both ways.
            for line in committed.lines() {
                let Some((assignment, note)) = line.split_once('#') else { continue };
                let Some((_, value)) = assignment.split_once('=') else { continue };
                let value = value.trim().trim_matches('"');
                if value.len() != 7 {
                    continue;
                }
                let c = Srgb::parse(value).expect("a colour");
                assert_eq!(
                    provenance(&scheme, c),
                    note.trim(),
                    "{slug}: {value} is annotated {}",
                    note.trim()
                );
            }
        }
    }
}
