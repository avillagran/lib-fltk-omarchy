//! The data a [`super::NavTree`] shows: items, badges, row actions,
//! search matches and what a custom row renderer receives.

use crate::text::ColorRole;
use crate::theme::{Rgb, Theme};
use std::ops::Range;

/// What an item is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavKind {
    /// A row: icon, title, detail, badges, actions; may have children.
    #[default]
    Row,
    /// A section header (caption, count, header actions); its children are
    /// the rows of the section. A hairline while the rail is collapsed.
    Section,
    /// A hairline between rows.
    Separator,
}

/// A mark on a row's title line, colored by a theme role.
#[derive(Debug, Clone, PartialEq)]
pub enum Badge {
    /// Text in `role`; `pill` draws it on a soft fill of the same role.
    Text {
        text: String,
        role: ColorRole,
        pill: bool,
    },
    /// A status dot.
    Dot(ColorRole),
    /// A small spinner (turns while animations are on).
    Spinner,
    /// A Nerd Font glyph in `role` (a pin, a lock); `label` is read by
    /// screen readers (translated by the app; may be empty).
    Icon {
        glyph: String,
        label: String,
        role: ColorRole,
    },
}

impl Badge {
    /// Plain text in a role ("12", "main").
    pub fn text(text: &str, role: ColorRole) -> Badge {
        Badge::Text {
            text: text.into(),
            role,
            pill: false,
        }
    }
    /// Text on a soft fill of its role ("PR #42", "failed").
    pub fn pill(text: &str, role: ColorRole) -> Badge {
        Badge::Text {
            text: text.into(),
            role,
            pill: true,
        }
    }
    pub fn dot(role: ColorRole) -> Badge {
        Badge::Dot(role)
    }
    pub fn spinner() -> Badge {
        Badge::Spinner
    }
    /// A glyph in a role (`Badge::icon(glyph::BOOKMARK, &tr!("Pinned"), ColorRole::Dim)`).
    pub fn icon(glyph: &str, label: &str, role: ColorRole) -> Badge {
        Badge::Icon {
            glyph: glyph.into(),
            label: label.into(),
            role,
        }
    }
}

/// How a row is laid out (see [`NavItem::card`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RowStyle {
    /// One line (icon, title, badges, trailing text or actions), plus the
    /// optional detail and usage lines.
    #[default]
    Line,
    /// o3's thread card, up to three lines: the icon, the
    /// [`NavItem::overline`], badges and the trailing text (or the hover
    /// actions) on top; the title and its [`NavItem::note`] in the
    /// middle; [`NavItem::meta`] (or the detail) at the bottom.
    Card,
}

/// One entry of a card's bottom line: a glyph and an optional text
/// (branch, host, provider). See [`NavItem::meta`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Meta {
    pub icon: String,
    pub text: String,
    /// Tint (default: dimmed text).
    pub role: Option<ColorRole>,
}

impl Meta {
    pub fn new(icon: &str, text: &str) -> Meta {
        Meta {
            icon: icon.into(),
            text: text.into(),
            role: None,
        }
    }
    pub fn role(mut self, role: ColorRole) -> Self {
        self.role = Some(role);
        self
    }
}

/// When a [`RowAction`] shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Show {
    /// Whenever the sidebar is expanded.
    #[default]
    Always,
    /// On the hovered, selected or keyboard-cursor row only.
    Hover,
}

/// A trailing icon button of a row or section header (eject, archive,
/// "+"). Clicks and the primary-action key (Delete, the first enabled
/// action) call [`super::NavTree::on_action`] with the item key and `id`.
#[derive(Debug, Clone, PartialEq)]
pub struct RowAction {
    pub id: String,
    pub icon: String,
    /// Tooltip and accessible name (from the app's catalog).
    pub tooltip: String,
    pub enabled: bool,
    pub show: Show,
}

impl RowAction {
    pub fn new(id: &str, icon: &str, tooltip: &str) -> RowAction {
        RowAction {
            id: id.into(),
            icon: icon.into(),
            tooltip: tooltip.into(),
            enabled: true,
            show: Show::Always,
        }
    }
    /// Only on the hovered, selected or cursor row.
    pub fn on_hover(mut self) -> Self {
        self.show = Show::Hover;
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
}

/// One entry of a [`super::NavTree`], kept in a
/// [`ListModel`](crate::model::ListModel) keyed by [`NavItem::key`]. The
/// tree comes from [`NavItem::parent`]: children follow their parent in
/// model order (an unknown parent puts the item at the top level).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NavItem {
    /// Stable identity (selection, cursor, folded state, motion).
    pub key: String,
    /// Key of the parent item; empty at the top level.
    pub parent: String,
    pub kind: NavKind,
    pub icon: String,
    /// Icon tint (default: the text color; the accent when active).
    pub icon_role: Option<ColorRole>,
    pub title: String,
    /// Dimmed second line, shown when expanded (the row grows).
    pub detail: String,
    /// Thin usage bar under the detail, 0..=1 (accent; urgent above 0.9).
    pub usage: Option<f64>,
    pub badges: Vec<Badge>,
    /// A shortcut hint at the end of the title line ("Ctrl+1").
    pub shortcut: String,
    /// A section's count ("12").
    pub count: String,
    /// Shown while a parent is collapsed ("+3 sub-threads"; the app
    /// translates it).
    pub summary: String,
    pub actions: Vec<RowAction>,
    /// Kept at the bottom, below the scrolling rows.
    pub footer: bool,
    /// A section that folds (rows with children always fold).
    pub collapsible: bool,
    /// Folded the first time it is seen.
    pub collapsed: bool,
    /// The current item (an open document, the running view): accent bar
    /// and icon, distinct from the selection.
    pub active: bool,
    /// Drawn dimmed but still activatable.
    pub dimmed: bool,
    /// Dimmed, not activatable, skipped by the keyboard.
    pub disabled: bool,
    /// A spinner in place of the actions (of the icon while collapsed).
    pub busy: bool,
    /// Unread / needs attention: a dot on the icon, bold title.
    pub attention: bool,
    /// Name of a custom renderer ([`super::NavTree::renderer`]).
    pub renderer: String,
    /// Built-in layout: one line (default) or an o3 card.
    pub style: RowStyle,
    /// A card's small top line (project, host), after the icon.
    pub overline: String,
    /// Text at the end of the first line ("3m", "Working 2m"): the
    /// hover-only actions take its place while the row is hovered or
    /// has the keyboard cursor.
    pub trailing: String,
    /// A glyph before [`NavItem::trailing`] (a state icon).
    pub trailing_icon: String,
    /// Tint of the trailing glyph and text (default: dimmed text).
    pub trailing_role: Option<ColorRole>,
    /// A dimmed text at the end of a card's title line (an age).
    pub note: String,
    /// A card's bottom line: glyphs and texts (branch, host, provider).
    /// The first entry with text takes the free width (elided in the
    /// middle); the entries after it are aligned to the end.
    pub meta: Vec<Meta>,
}

impl NavItem {
    /// A row.
    pub fn row(key: &str, icon: &str, title: &str) -> NavItem {
        NavItem {
            key: key.into(),
            icon: icon.into(),
            title: title.into(),
            ..Default::default()
        }
    }
    /// A section header; give its rows `.parent(key)`.
    pub fn section(key: &str, title: &str) -> NavItem {
        NavItem {
            key: key.into(),
            title: title.into(),
            kind: NavKind::Section,
            ..Default::default()
        }
    }
    /// A hairline.
    pub fn separator(key: &str) -> NavItem {
        NavItem {
            key: key.into(),
            kind: NavKind::Separator,
            ..Default::default()
        }
    }
    pub fn parent(mut self, key: &str) -> Self {
        self.parent = key.into();
        self
    }
    pub fn icon_role(mut self, role: ColorRole) -> Self {
        self.icon_role = Some(role);
        self
    }
    pub fn detail(mut self, d: &str) -> Self {
        self.detail = d.into();
        self
    }
    pub fn usage(mut self, fraction: f64) -> Self {
        self.usage = fraction.is_finite().then(|| fraction.clamp(0.0, 1.0));
        self
    }
    pub fn badge(mut self, b: Badge) -> Self {
        self.badges.push(b);
        self
    }
    pub fn shortcut(mut self, s: &str) -> Self {
        self.shortcut = s.into();
        self
    }
    pub fn count(mut self, c: &str) -> Self {
        self.count = c.into();
        self
    }
    pub fn summary(mut self, s: &str) -> Self {
        self.summary = s.into();
        self
    }
    pub fn action(mut self, a: RowAction) -> Self {
        self.actions.push(a);
        self
    }
    pub fn footer(mut self) -> Self {
        self.footer = true;
        self
    }
    pub fn collapsible(mut self) -> Self {
        self.collapsible = true;
        self
    }
    pub fn collapsed(mut self) -> Self {
        self.collapsed = true;
        self
    }
    pub fn active(mut self, on: bool) -> Self {
        self.active = on;
        self
    }
    pub fn dimmed(mut self) -> Self {
        self.dimmed = true;
        self
    }
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
    pub fn busy(mut self, on: bool) -> Self {
        self.busy = on;
        self
    }
    pub fn attention(mut self, on: bool) -> Self {
        self.attention = on;
        self
    }
    pub fn renderer(mut self, name: &str) -> Self {
        self.renderer = name.into();
        self
    }
    /// Lays the row out as an o3 thread card ([`RowStyle::Card`]).
    pub fn card(mut self) -> Self {
        self.style = RowStyle::Card;
        self
    }
    pub fn overline(mut self, text: &str) -> Self {
        self.overline = text.into();
        self
    }
    /// Text at the end of the first line; hover-only actions replace it.
    pub fn trailing(mut self, text: &str) -> Self {
        self.trailing = text.into();
        self
    }
    /// A state glyph and text at the end of the first line, in `role`
    /// ("Working 2m" in yellow); hover-only actions replace it.
    pub fn status(mut self, icon: &str, text: &str, role: ColorRole) -> Self {
        self.trailing_icon = icon.into();
        self.trailing = text.into();
        self.trailing_role = Some(role);
        self
    }
    pub fn note(mut self, text: &str) -> Self {
        self.note = text.into();
        self
    }
    pub fn meta(mut self, m: Meta) -> Self {
        self.meta.push(m);
        self
    }
}

/// A search hit: a score (higher first) and byte ranges of
/// [`NavItem::title`] to highlight.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Match {
    pub score: i32,
    pub ranges: Vec<Range<usize>>,
}

fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// The default matcher of the search field: every word of `query` must
/// be found in the title (case-insensitive), else in the detail. Matches
/// at the start of the title rank first, then at a word start, then
/// inside a word; the title ranks above the detail.
pub fn default_match(item: &NavItem, query: &str) -> Option<Match> {
    if item.kind != NavKind::Row {
        return None;
    }
    let title = fold(&item.title);
    let detail = fold(&item.detail);
    // Lowercasing can change byte lengths (rare); highlight only when it did not.
    let same = title.len() == item.title.len();
    let mut score = 0;
    let mut ranges = Vec::new();
    let mut any = false;
    for word in query.split_whitespace().map(fold) {
        any = true;
        if let Some(at) = title.find(&word) {
            let start_of_word = at == 0
                || title[..at]
                    .chars()
                    .last()
                    .is_some_and(|c| !c.is_alphanumeric());
            score += if at == 0 {
                100
            } else if start_of_word {
                75
            } else {
                45
            };
            if same {
                ranges.push(at..at + word.len());
            }
        } else if detail.contains(&word) {
            score += 20;
        } else {
            return None;
        }
    }
    any.then_some(Match { score, ranges })
}

/// The state a custom renderer draws.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RowState {
    pub selected: bool,
    pub active: bool,
    pub hover: bool,
    /// The keyboard cursor row, with the focus ring shown.
    pub focused: bool,
    pub drop_target: bool,
    pub depth: usize,
    /// Some(open) for items that fold.
    pub expanded: Option<bool>,
    /// 0 at rail width .. 1 expanded (labels fade in with it).
    pub reveal: f64,
    pub dimmed: bool,
    pub disabled: bool,
}

/// What a custom renderer gets: the item, its rectangle (window
/// coordinates), its state, the theme and the color under it (the
/// selection or hover fill is already painted). Paint with
/// [`crate::paint`] and theme colors; mirror with `rtl`.
pub struct RowPaint<'a> {
    pub item: &'a NavItem,
    pub rect: (i32, i32, i32, i32),
    pub state: RowState,
    pub theme: &'a Theme,
    pub under: Rgb,
    pub rtl: bool,
}
