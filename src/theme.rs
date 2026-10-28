//! Omarchy theme: palette from `colors.toml`, surface roles and the
//! typography/spacing/state tokens from `shell.toml` (theme file, then the
//! user's `~/.config/omarchy/shell.toml`), corner radius and gaps from
//! Hyprland — the same sources `omarchy-shell` uses (Commons/Color.qml,
//! Commons/Style.qml).
//!
//! The current theme lives in a thread-local; components read it at draw
//! time, so a theme switch is `theme::set(..)` + redraw.

use crate::toml::{self, Table};
use fltk::enums::Color;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// 0xRRGGBB color with helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u32);

impl Rgb {
    pub fn parse(s: &str) -> Option<Rgb> {
        let h = s.trim().strip_prefix('#')?;
        let h = match h.len() {
            3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
            6 | 8 => h[..6].to_string(),
            _ => return None,
        };
        u32::from_str_radix(&h, 16).ok().map(Rgb)
    }
    pub fn r(self) -> u8 { (self.0 >> 16) as u8 }
    pub fn g(self) -> u8 { (self.0 >> 8) as u8 }
    pub fn b(self) -> u8 { self.0 as u8 }

    /// `self` painted over `bg` with opacity `alpha` (Omarchy's alpha fills).
    pub fn over(self, bg: Rgb, alpha: f64) -> Rgb {
        let a = alpha.clamp(0.0, 1.0);
        let mix = |f: u8, b: u8| ((b as f64) + (f as f64 - b as f64) * a).round() as u32;
        Rgb((mix(self.r(), bg.r()) << 16) | (mix(self.g(), bg.g()) << 8) | mix(self.b(), bg.b()))
    }

    pub fn fl(self) -> Color {
        Color::from_rgb(self.r(), self.g(), self.b())
    }

    pub fn hex(self) -> String {
        format!("#{:06x}", self.0)
    }

    /// Qt.darker: HSV value divided by `factor` (hue and saturation kept).
    pub fn darker(self, factor: f64) -> Rgb {
        let (r, g, b) = (self.r() as f64, self.g() as f64, self.b() as f64);
        let max = r.max(g).max(b);
        if max <= 0.0 || factor <= 0.0 {
            return self;
        }
        let k = (max / factor) / max;
        let c = |x: f64| ((x * k).round().clamp(0.0, 255.0)) as u32;
        Rgb((c(r) << 16) | (c(g) << 8) | c(b))
    }

    /// Relative luminance (0..1), for choosing readable text on a fill.
    pub fn luminance(self) -> f64 {
        (0.2126 * self.r() as f64 + 0.7152 * self.g() as f64 + 0.0722 * self.b() as f64) / 255.0
    }
}

/// Foundational palette (`colors.toml`).
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub background: Rgb,
    pub foreground: Rgb,
    pub accent: Rgb,
    /// Attention color (Omarchy "urgent"): `colors.toml` red by default.
    pub urgent: Rgb,
    pub selection: Rgb,
    pub muted: Rgb,
    pub dim: Rgb,
    pub red: Rgb,
    pub green: Rgb,
    pub yellow: Rgb,
    pub blue: Rgb,
    pub cyan: Rgb,
    pub magenta: Rgb,
    pub light: bool,
}

/// A surface's colors (`[popups]`, `[menu]`, `[tooltip]`, `[notifications]`…).
#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    pub background: Rgb,
    pub background_alpha: f64,
    pub text: Rgb,
    pub border: Rgb,
    pub border_alpha: f64,
    pub border_width: i32,
    pub selected_background: Rgb,
    pub selected_background_alpha: f64,
    pub selected_text: Rgb,
    pub selected_border: Rgb,
    pub selected_border_alpha: f64,
}

impl Surface {
    /// Card fill as an opaque color (alpha flattened over the window background).
    pub fn fill(&self, under: Rgb) -> Rgb {
        self.background.over(under, self.background_alpha)
    }
}

/// One interactive state (`[controls]` normal / hover-cursor / focus / selected).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateStyle {
    pub color: Rgb,
    pub fill_alpha: f64,
    pub border: Rgb,
    pub border_width: i32,
    pub border_alpha: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Controls {
    pub normal: StateStyle,
    pub hover: StateStyle,
    pub focus: StateStyle,
    pub selected: StateStyle,
    pub pressed_fill_alpha: f64,
    pub selection_fill_alpha: f64,
}

/// Typography scale (Style.font.*), in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fonts {
    pub base: i32,
    pub caption: i32,
    pub body_small: i32,
    pub body: i32,
    pub subtitle: i32,
    pub title: i32,
    pub heading: i32,
    pub display: i32,
    pub display_large: i32,
    pub icon_small: i32,
    pub icon: i32,
    pub icon_large: i32,
}

/// Spacing scale (Style.spacing.*), in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing {
    pub hairline: i32,
    pub xxs: i32,
    pub xs: i32,
    pub sm: i32,
    pub md: i32,
    pub lg: i32,
    pub xl: i32,
    pub xxl: i32,
    pub xxxl: i32,
    pub huge: i32,
    pub control_gap: i32,
    pub control_padding_x: i32,
    pub control_padding_y: i32,
    pub input_padding_y: i32,
    pub control_height: i32,
    pub popup_row_height: i32,
    pub dropdown_width: i32,
    pub searchable_dropdown_width: i32,
    pub number_field_width: i32,
    pub row_gap: i32,
    pub row_padding_x: i32,
    pub label_gap: i32,
    pub panel_gap: i32,
    pub panel_padding: i32,
    pub popup_padding: i32,
}

/// Interior corner style, an app preference (Files: `corners = "omarchy" | "square" | "rounded"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Corners {
    /// Follow Hyprland `decoration:rounding` (what omarchy-shell does).
    #[default]
    Omarchy,
    /// Always square.
    Square,
    /// Always rounded (8 px at base size 12, or Hyprland's radius if larger).
    Rounded,
}

impl Corners {
    pub fn parse(s: &str) -> Option<Corners> {
        match s.trim().to_ascii_lowercase().as_str() {
            "omarchy" | "auto" => Some(Corners::Omarchy),
            "square" => Some(Corners::Square),
            "rounded" | "round" => Some(Corners::Rounded),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Corners::Omarchy => "omarchy",
            Corners::Square => "square",
            Corners::Rounded => "rounded",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: String,
    pub palette: Palette,
    pub controls: Controls,
    pub popups: Surface,
    pub menu: Surface,
    pub tooltip: Surface,
    pub notifications: Surface,
    pub font: Fonts,
    pub spacing: Spacing,
    /// Effective interior corner radius (from `corners` and Hyprland).
    pub radius: i32,
    /// Raw Hyprland `decoration:rounding` (Style.cornerRadius).
    pub hypr_radius: i32,
    pub corners: Corners,
    /// Half of Hyprland `general:gaps_out` (Style.gapsOut).
    pub gaps_out: i32,
    /// Font family; "monospace" follows `omarchy-font-set` through fontconfig.
    pub font_family: String,
}

// ------------------------------------------------------------------ loading

pub fn omarchy_state_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("OMARCHY_FLTK_STATE_DIR") {
        return PathBuf::from(p);
    }
    home().join(".local/state/omarchy/current")
}

pub fn theme_dir() -> PathBuf {
    omarchy_state_dir().join("theme")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn user_shell_toml() -> PathBuf {
    if let Some(p) = std::env::var_os("OMARCHY_FLTK_USER_SHELL") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"));
    base.join("omarchy/shell.toml")
}

fn read(path: &Path) -> Table {
    std::fs::read_to_string(path).map(|t| toml::parse(&t)).unwrap_or_default()
}

/// Built-in fallback when Omarchy is not installed (Omarchy's default dark look).
pub const FALLBACK_COLORS: &str = r##"
accent = "#7aa2f7"
background = "#1a1b26"
foreground = "#a9b1d6"
selection = "#2a2f45"
red = "#f7768e"
green = "#9ece6a"
yellow = "#e0af68"
blue = "#7aa2f7"
magenta = "#bb9af7"
cyan = "#7dcfff"
"##;

impl Theme {
    /// Loads the current Omarchy theme, or the built-in fallback.
    pub fn load() -> Theme {
        let dir = theme_dir();
        let colors = read(&dir.join("colors.toml"));
        let colors = if colors.is_empty() { toml::parse(FALLBACK_COLORS) } else { colors };
        let mut shell = read(&dir.join("shell.toml"));
        shell.extend(read(&user_shell_toml())); // user keys win
        let name = std::fs::read_to_string(omarchy_state_dir().join("theme.name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "Omarchy".into());
        let (radius, gaps) = if crate::hyprland::available() {
            (
                crate::hyprland::option_int("decoration:rounding").unwrap_or(0) as i32,
                crate::hyprland::gaps_out().map(|g| (g as f64 / 2.0).round() as i32).unwrap_or(5),
            )
        } else {
            (0, 5)
        };
        Theme::from_tables(&name, &colors, &shell, radius, gaps)
    }

    /// Builds a theme from parsed `colors.toml` and merged `shell.toml` tables.
    pub fn from_tables(name: &str, colors: &Table, shell: &Table, radius: i32, gaps_out: i32) -> Theme {
        let col = |k: &str| colors.get(k).and_then(|v| v.as_str()).and_then(Rgb::parse);
        // Same keys, fallbacks and defaults as omarchy-shell Commons/Color.qml.
        let background = col("background").or(col("color0")).unwrap_or(Rgb(0x101315));
        let foreground = col("foreground").or(col("color7")).unwrap_or(Rgb(0xcacccc));
        let accent = col("accent").or(col("color4")).unwrap_or(Rgb(0xcacccc));
        let urgent = col("red").or(col("color1")).unwrap_or(Rgb(0xa55555));
        let light = colors.get("mode").and_then(|v| v.as_str()) == Some("light")
            || (colors.get("mode").is_none() && background.luminance() > 0.5);
        let palette = Palette {
            background,
            foreground,
            accent,
            urgent,
            selection: col("selection").unwrap_or(foreground.over(background, 0.18)),
            muted: col("muted").or(col("color8")).unwrap_or(Rgb(0x707880)),
            dim: foreground.darker(1.4),
            red: col("red").unwrap_or(urgent),
            green: col("green").unwrap_or(Rgb(0x8fbf6f)),
            yellow: col("yellow").unwrap_or(Rgb(0xe0af68)),
            blue: col("blue").unwrap_or(accent),
            cyan: col("cyan").unwrap_or(foreground),
            magenta: col("magenta").unwrap_or(accent),
            light,
        };

        let hypr_active = shell_color(shell, "hyprland.active-border", &palette)
            .or_else(|| shell_color(shell, "notifications.border", &palette))
            .unwrap_or(accent);
        let hypr_active_fg = shell_color(shell, "hyprland.active-border-foreground", &palette).unwrap_or(foreground);
        let resolve = |key: &str, fallback: Rgb| -> Rgb {
            match shell.get(key).and_then(|v| v.as_str()) {
                Some("hyprland.active-border") => hypr_active,
                Some("hyprland.active-border-foreground") => hypr_active_fg,
                Some(s) => role_or_hex(s, &palette).unwrap_or(fallback),
                None => fallback,
            }
        };
        let num = |key: &str, fallback: f64| shell.get(key).and_then(|v| v.as_f64()).unwrap_or(fallback);

        let state = |name: &str, fill: f64, bw: f64, ba: f64, base: Option<StateStyle>| -> StateStyle {
            let color = resolve(&format!("controls.{name}-color"), base.map(|b| b.color).unwrap_or(foreground));
            StateStyle {
                color,
                fill_alpha: num(&format!("controls.{name}-fill-alpha"), base.map(|b| b.fill_alpha).unwrap_or(fill)),
                border: resolve(&format!("controls.{name}-border"), color),
                border_width: num(&format!("controls.{name}-border-width"), base.map(|b| b.border_width as f64).unwrap_or(bw)).round() as i32,
                border_alpha: num(&format!("controls.{name}-border-alpha"), base.map(|b| b.border_alpha).unwrap_or(ba)),
            }
        };
        let normal = state("normal", 0.04, 1.0, 0.4, None);
        let hover = state("hover-cursor", 0.08, normal.border_width as f64, 0.25, None);
        let controls = Controls {
            normal,
            hover,
            focus: state("focus", 0.0, 0.0, 0.0, Some(hover)),
            selected: state("selected", 0.18, 0.0, 1.0, None),
            pressed_fill_alpha: num("controls.pressed-fill-alpha", 0.22),
            selection_fill_alpha: num("controls.selection-fill-alpha", 0.35),
        };

        let surface = |sec: &str, border_default: Rgb, bg_alpha: f64, sel_border_alpha: f64| -> Surface {
            let k = |x: &str| format!("{sec}.{x}");
            let text = resolve(&k("text"), foreground);
            Surface {
                background: resolve(&k("background"), background),
                background_alpha: num(&k("background-alpha"), bg_alpha),
                text,
                border: resolve(&k("border"), border_default),
                border_alpha: num(&k("border-alpha"), 1.0),
                border_width: num(&k("border-width"), 2.0).round() as i32,
                selected_background: resolve(&k("selected-background"), foreground),
                selected_background_alpha: num(&k("selected-background-alpha"), 0.08),
                selected_text: resolve(&k("selected-text"), accent),
                selected_border: resolve(&k("selected-border"), hypr_active_fg),
                selected_border_alpha: num(&k("selected-border-alpha"), sel_border_alpha),
            }
        };

        let base = num("font.base-size", 12.0).max(1.0);
        let fpx = |key: &str, mult: f64| -> i32 {
            shell.get(&format!("font.{key}")).and_then(|v| v.as_f64()).map(|v| v.round() as i32).unwrap_or(((base * mult).round() as i32).max(1))
        };
        let body_small = fpx("body-small", 0.917);
        let title = fpx("title", 1.167);
        let font = Fonts {
            base: base as i32,
            caption: fpx("caption", 0.833),
            body_small,
            body: fpx("body", 1.0),
            subtitle: fpx("subtitle", 1.083),
            title,
            heading: fpx("heading", 1.333),
            display: fpx("display", 2.0),
            display_large: fpx("display-large", 2.333),
            icon_small: shell.get("font.icon-small").and_then(|v| v.as_f64()).map(|v| v as i32).unwrap_or(body_small),
            icon: shell.get("font.icon").and_then(|v| v.as_f64()).map(|v| v as i32).unwrap_or(title),
            icon_large: fpx("icon-large", 1.5),
        };

        let font_scale = (base / 12.0).max(1.0 / 12.0);
        let with_font = shell.get("spacing.scale-with-font").and_then(|v| v.as_bool()).unwrap_or(true);
        let scale = num("spacing.scale", 1.0).max(0.0) * if with_font { font_scale } else { 1.0 };
        let sp = |key: &str, px: f64| -> i32 {
            if let Some(v) = shell.get(&format!("spacing.{key}")).and_then(|v| v.as_f64()) {
                return v.round().max(0.0) as i32;
            }
            let n = px * scale;
            if n <= 0.0 { 0 } else { (n.round() as i32).max(1) }
        };
        let spacing = Spacing {
            hairline: sp("hairline", 1.0),
            xxs: sp("xxs", 2.0),
            xs: sp("xs", 3.0),
            sm: sp("sm", 4.0),
            md: sp("md", 6.0),
            lg: sp("lg", 8.0),
            xl: sp("xl", 10.0),
            xxl: sp("xxl", 12.0),
            xxxl: sp("xxxl", 14.0),
            huge: sp("huge", 18.0),
            control_gap: sp("control-gap", 8.0),
            control_padding_x: sp("control-padding-x", 10.0),
            control_padding_y: sp("control-padding-y", 6.0),
            input_padding_y: sp("input-padding-y", 7.0),
            control_height: sp("control-height", 28.0),
            popup_row_height: sp("popup-row-height", 28.0),
            dropdown_width: sp("dropdown-width", 240.0),
            searchable_dropdown_width: sp("searchable-dropdown-width", 260.0),
            number_field_width: sp("number-field-width", 120.0),
            row_gap: sp("row-gap", 8.0),
            row_padding_x: sp("row-padding-x", 12.0),
            label_gap: sp("label-gap", 4.0),
            panel_gap: sp("panel-gap", 14.0),
            panel_padding: sp("panel-padding", 18.0),
            popup_padding: sp("popup-padding", 14.0),
        };

        Theme {
            name: name.to_string(),
            controls,
            // Defaults from Color.qml: popups/notifications border accent,
            // menu/tooltip border foreground, menu selected-border alpha 0.
            popups: surface("popups", accent, 1.0, 0.25),
            menu: surface("menu", foreground, 1.0, 0.0),
            tooltip: { let mut t = surface("tooltip", foreground, 1.0, 0.25); t.border_width = num("tooltip.border-width", 1.0) as i32; t },
            notifications: surface("notifications", accent, 1.0, 0.25),
            palette,
            font,
            spacing,
            radius,
            hypr_radius: radius,
            corners: Corners::Omarchy,
            gaps_out,
            font_family: std::env::var("OMARCHY_FLTK_FONT").unwrap_or_else(|_| "monospace".into()),
        }
    }

    /// Same theme with another corner preference.
    pub fn with_corners(mut self, corners: Corners) -> Theme {
        self.corners = corners;
        let rounded = ((8.0 * self.font.base as f64 / 12.0).round() as i32).max(self.hypr_radius);
        self.radius = match corners {
            Corners::Omarchy => self.hypr_radius,
            Corners::Square => 0,
            Corners::Rounded => rounded,
        };
        self
    }

    /// Fixed radius for thumbnails, type badges and previews: always
    /// curved, independent of the corner preference.
    pub fn badge_radius(&self) -> i32 {
        (8.0 * self.font.base as f64 / 12.0).round() as i32
    }

    // ---- resolved state colors over a given surface background ----

    pub fn fill(&self, s: &StateStyle, under: Rgb) -> Rgb {
        s.color.over(under, s.fill_alpha)
    }

    pub fn border(&self, s: &StateStyle, under: Rgb) -> Rgb {
        s.border.over(under, s.border_alpha)
    }
}

fn role_or_hex(s: &str, p: &Palette) -> Option<Rgb> {
    match s.trim().to_ascii_lowercase().as_str() {
        "foreground" | "text" => Some(p.foreground),
        "accent" => Some(p.accent),
        "urgent" => Some(p.urgent),
        "background" => Some(p.background),
        _ => Rgb::parse(s),
    }
}

/// Hyprland-style values may be gradients ("rgba(..) rgba(..) 45deg"); take the first color.
fn shell_color(shell: &Table, key: &str, p: &Palette) -> Option<Rgb> {
    let s = shell.get(key)?.as_str()?;
    if let Some(rest) = s.trim().strip_prefix("rgba(").or_else(|| s.trim().strip_prefix("rgb(")) {
        let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        return Rgb::parse(&format!("#{}", &hex[..hex.len().min(6)]));
    }
    role_or_hex(s.split_whitespace().next()?, p)
}

// ---------------------------------------------------------------- current

thread_local! {
    static CORNERS: std::cell::Cell<Corners> = const { std::cell::Cell::new(Corners::Omarchy) };
    static CURRENT: RefCell<Option<Theme>> = const { RefCell::new(None) };
    static LISTENERS: RefCell<Vec<Box<dyn FnMut(&Theme)>>> = const { RefCell::new(Vec::new()) };
}

/// The active theme (loads it on first use).
pub fn current() -> Theme {
    with(Theme::clone)
}

/// Reads one value of the active theme without cloning it.
pub fn with<R>(f: impl FnOnce(&Theme) -> R) -> R {
    CURRENT.with(|c| {
        if c.borrow().is_none() {
            let t = Theme::load().with_corners(CORNERS.with(|k| k.get()));
            *c.borrow_mut() = Some(t);
        }
        // Shared borrow: nested `with` calls (draw helpers) are fine.
        f(c.borrow().as_ref().expect("theme loaded"))
    })
}

/// Makes `theme` active: FLTK colormap, default fonts, every window redrawn.
pub fn set(theme: Theme) {
    let theme = theme.with_corners(CORNERS.with(|k| k.get()));
    apply_to_fltk(&theme);
    CURRENT.with(|c| *c.borrow_mut() = Some(theme.clone()));
    // Taken out while running so a listener may register another one.
    let mut list = LISTENERS.with(|l| std::mem::take(&mut *l.borrow_mut()));
    list.iter_mut().for_each(|f| f(&theme));
    LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        let added = std::mem::take(&mut *l);
        *l = list;
        l.extend(added);
    });
    fltk::app::redraw();
}

/// Sets the interior corner preference and restyles every window.
/// It survives theme reloads.
pub fn set_corners(corners: Corners) {
    CORNERS.with(|k| k.set(corners));
    set(current());
}

pub fn corners() -> Corners {
    CORNERS.with(|k| k.get())
}

/// Called after every theme change (on the UI thread).
pub fn on_change(f: impl FnMut(&Theme) + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(f)));
}

/// Re-reads the Omarchy theme; applies it if it changed. Returns true on change.
pub fn reload() -> bool {
    let fresh = Theme::load().with_corners(CORNERS.with(|k| k.get()));
    if with(|t| *t == fresh) {
        return false;
    }
    set(fresh);
    true
}

fn apply_to_fltk(t: &Theme) {
    use fltk::app;
    use fltk::enums::Font;
    let p = &t.palette;
    let bg = p.background;
    app::background(bg.r(), bg.g(), bg.b());
    let field = t.fill(&t.controls.normal, bg);
    app::background2(field.r(), field.g(), field.b());
    app::foreground(p.foreground.r(), p.foreground.g(), p.foreground.b());
    let sel = p.foreground.over(bg, t.controls.selection_fill_alpha);
    app::set_color(Color::Selection, sel.r(), sel.g(), sel.b());
    let inactive = p.foreground.over(bg, 0.45);
    app::set_color(Color::Inactive, inactive.r(), inactive.g(), inactive.b());
    Font::set_font(Font::Helvetica, &t.font_family);
    Font::set_font(Font::HelveticaBold, &format!("{} Bold", t.font_family));
    Font::set_font(Font::Courier, &t.font_family);
    Font::set_font(Font::CourierBold, &format!("{} Bold", t.font_family));
    app::set_font_size(t.font.body);
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLORS: &str = "accent = \"#e68e0d\"\nbackground = \"#121212\"\nforeground = \"#bebebe\"\nred = \"#D35F5F\"\n";

    #[test]
    fn defaults_follow_omarchy_style_tokens() {
        let t = Theme::from_tables("x", &toml::parse(COLORS), &Table::new(), 0, 5);
        assert_eq!(t.palette.accent, Rgb(0xe68e0d));
        assert_eq!(t.palette.urgent, Rgb(0xd35f5f));
        assert_eq!(t.controls.normal.fill_alpha, 0.04);
        assert_eq!(t.controls.hover.border_alpha, 0.25);
        assert_eq!(t.controls.focus, t.controls.hover);
        assert_eq!(t.controls.selected.border_width, 0);
        assert_eq!((t.font.body, t.font.title, t.font.heading, t.font.display), (12, 14, 16, 24));
        assert_eq!((t.spacing.control_height, t.spacing.panel_padding), (28, 18));
    }

    #[test]
    fn shell_toml_overrides_and_font_scale() {
        let shell = toml::parse("[font]\nbase-size = 15\ntitle = 20\n[controls]\nnormal-fill-alpha = 0.1\n[menu]\nborder = \"hyprland.active-border\"\n[hyprland]\nactive-border = \"rgba(33ccffee) rgba(00ff99ee) 45deg\"\n[spacing]\ncontrol-height = 40\n");
        let t = Theme::from_tables("x", &toml::parse(COLORS), &shell, 8, 5);
        assert_eq!(t.font.body, 15);
        assert_eq!(t.font.title, 20);
        assert_eq!(t.spacing.panel_padding, 23); // 18 * 15/12
        assert_eq!(t.spacing.control_height, 40);
        assert_eq!(t.controls.normal.fill_alpha, 0.1);
        assert_eq!(t.menu.border, Rgb(0x33ccff));
    }

    #[test]
    fn corner_preference() {
        let t = Theme::from_tables("x", &toml::parse(COLORS), &Table::new(), 0, 5);
        assert_eq!(t.clone().with_corners(Corners::Rounded).radius, 8);
        assert_eq!(t.clone().with_corners(Corners::Square).radius, 0);
        let t10 = Theme::from_tables("x", &toml::parse(COLORS), &Table::new(), 10, 5);
        assert_eq!(t10.clone().with_corners(Corners::Omarchy).radius, 10);
        assert_eq!(t10.with_corners(Corners::Square).radius, 0);
        assert_eq!(Corners::parse("rounded"), Some(Corners::Rounded));
        assert_eq!(t.badge_radius(), 8);
    }

    #[test]
    fn alpha_over() {
        assert_eq!(Rgb(0xffffff).over(Rgb(0x000000), 0.5), Rgb(0x808080));
        assert_eq!(Rgb(0xbebebe).over(Rgb(0x121212), 0.04), Rgb(0x191919));
    }
}
