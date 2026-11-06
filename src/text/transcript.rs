//! Types for hosting a chat transcript in a [`RichText`](super::RichText):
//! keyed messages, app-drawn embeds, block actions, scroll anchors and
//! images (see `RichText::set_message`, `embed_painter`, `code_actions`,
//! `scroll_anchor`, `on_image`).

use super::doc::Block;
use crate::theme::{Rgb, Theme};

/// The content of one message (a transcript row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Markdown, parsed by the view. Setting a value that extends the one
    /// set before streams only the new text (O(tail)).
    Markdown(String),
    /// Blocks built in code (cards are usually one [`Block::embed`]).
    Blocks(Vec<Block>),
}

impl Message {
    pub fn markdown(md: &str) -> Message {
        Message::Markdown(md.to_string())
    }

    pub fn blocks(blocks: impl IntoIterator<Item = Block>) -> Message {
        Message::Blocks(blocks.into_iter().collect())
    }

    /// One app-drawn block (an activity card, a plan, an approval).
    pub fn embed(id: &str, height: i32, text: &str) -> Message {
        Message::Blocks(vec![Block::embed(id, height, text)])
    }
}

/// An action of a code block header or a table footer: `id` comes back in
/// [`BlockActionEvent::action`]; `label` is shown (translate it with
/// `tr!`: actions are asked for again after a language change).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockAction {
    pub id: String,
    pub label: String,
}

impl BlockAction {
    /// The code block copy button (built in, always there).
    pub const COPY: &'static str = "copy";
    /// Built-in table action: copies the table as GFM Markdown.
    pub const COPY_MARKDOWN: &'static str = "copy-markdown";
    /// Built-in table action: copies the table as CSV.
    pub const COPY_CSV: &'static str = "copy-csv";

    pub fn new(id: &str, label: &str) -> BlockAction {
        BlockAction {
            id: id.to_string(),
            label: label.to_string(),
        }
    }
}

/// A block action was used (click, Enter or Space). Built-in copies have
/// already reached the clipboard when this arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockActionEvent {
    /// The action id ([`BlockAction::COPY`], `COPY_MARKDOWN`, `COPY_CSV`
    /// or one of the app's).
    pub action: String,
    /// Block index in the document.
    pub block: usize,
    /// Key of the message that holds the block, and the block's index in it
    /// (o3: the n-th code block of a message maps to its command metadata).
    pub message: String,
    pub index: usize,
    /// Code block language (None for tables).
    pub lang: Option<String>,
    /// The code; for a table its Markdown (its CSV for `COPY_CSV`).
    pub text: String,
}

/// What the embed painter gets for one embed in view (window
/// coordinates; clipped to the view). Draw with `fltk::draw` / the
/// toolkit's `paint` helpers and theme colors.
pub struct EmbedPaint<'a> {
    pub id: &'a str,
    /// The embed's text (or an image's alt text).
    pub text: &'a str,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub hovered: bool,
    /// Focused with Tab (draw a ring: `crate::focus` style).
    pub focused: bool,
    /// Inside the selection.
    pub selected: bool,
    /// An image resolved with [`ImageData::Painted`].
    pub image: bool,
    pub theme: &'a Theme,
    /// The color under the embed.
    pub under: Rgb,
}

/// Where a transcript is scrolled, by message key: save it when leaving a
/// thread, restore it when coming back (`RichText::scroll_anchor` /
/// `restore_scroll`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScrollAnchor {
    /// Key of the message at the top of the view.
    pub key: String,
    /// Pixels from that message's top to the view's top.
    pub offset: i64,
    /// The view showed the end (and followed it).
    pub at_end: bool,
}

/// The app's answer for an image source (`RichText::set_image`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageData {
    /// Size known, pixels not yet: a placeholder of that size.
    Size { width: i32, height: i32 },
    /// Decoded pixels, RGBA8, `width * height * 4` bytes. Shown at most
    /// the column wide and `image_max_height` tall.
    Rgba { width: i32, height: i32, pixels: Vec<u8> },
    /// The embed painter draws it (`EmbedPaint::image`), this tall.
    Painted { height: i32 },
    /// Could not load: the alt text stays.
    Failed,
}
