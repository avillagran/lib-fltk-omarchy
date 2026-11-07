//! [`Composer`]: chat input on a [`TextArea`].

use super::area::{Hook, TextArea};
use crate::a11y::{self, Action, Role};
use crate::layout::FlexExt;
use crate::model::ListModel;
use crate::overlay::{bg, card, close_layer, open_layer};
use crate::paint;
use crate::state::{State, StateFeed};
use crate::theme;
use crate::widgets::{glyph, Button};
use fltk::{app, enums::*, group::Flex, prelude::*, widget::Widget};
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

/// Nerd Font "send" (paper plane) used by the send button.
pub const SEND: &str = "\u{f048a}";
/// Nerd Font paperclip of the attach button ([`Composer::attach_button`]).
pub const ATTACH: &str = "\u{f03e2}";

/// Up/Down through the prompts sent before (newest first), o3's rules:
/// the first step up keeps the draft being written, stepping back down
/// past the newest prompt restores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptHistory {
    /// -1: the draft; 0: the newest prompt.
    index: isize,
    scratch: String,
}

impl Default for PromptHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl PromptHistory {
    pub fn new() -> Self {
        PromptHistory {
            index: -1,
            scratch: String::new(),
        }
    }

    /// The text to show after one step (`up`: older), or None when there
    /// is nowhere to go. `draft` is the text now in the input.
    pub fn step(&mut self, list: &[String], up: bool, draft: &str) -> Option<String> {
        if up {
            if self.index + 1 >= list.len() as isize {
                return None;
            }
            if self.index < 0 {
                self.scratch = draft.to_string();
            }
            self.index += 1;
            Some(list[self.index as usize].clone())
        } else {
            if self.index < 0 {
                return None;
            }
            self.index -= 1;
            Some(if self.index < 0 {
                std::mem::take(&mut self.scratch)
            } else {
                list[self.index as usize].clone()
            })
        }
    }

    /// Showing a prompt of the list (not the draft).
    pub fn browsing(&self) -> bool {
        self.index >= 0
    }

    /// Back to the draft's place (the user edited, sent, or the list
    /// changed).
    pub fn reset(&mut self) {
        self.index = -1;
        self.scratch.clear();
    }
}

/// Where a trigger character starts a completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// At the start of a word (`@files`, `$skills`): after a blank or at
    /// the start of the text.
    WordStart,
    /// Only at the start of a line (`/commands`).
    LineStart,
}

/// One suggestion of a completion list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompletionItem {
    /// Nerd Font glyph (optional).
    pub icon: String,
    pub label: String,
    /// Dimmed text after the label (a path, a description).
    pub detail: String,
    /// What replaces the trigger and the query. Empty: the trigger char,
    /// the label and a space.
    pub insert: String,
    /// When set, the inserted text becomes a chip carrying this data.
    pub chip: Option<String>,
}

impl CompletionItem {
    pub fn new(label: &str) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            ..Default::default()
        }
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = glyph.into();
        self
    }
    pub fn detail(mut self, text: &str) -> Self {
        self.detail = text.into();
        self
    }
    /// Exact replacement text (no space is added).
    pub fn insert(mut self, text: &str) -> Self {
        self.insert = text.into();
        self
    }
    /// Inserted as an atomic chip carrying `data` (a path, an ID).
    pub fn chip(mut self, data: &str) -> Self {
        self.chip = Some(data.into());
        self
    }
}

/// The text being completed: `trigger`, the `text` typed after it, and
/// the byte `range` (trigger included) that an accepted item replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub trigger: char,
    pub text: String,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
enum Status {
    Loading,
    Items(Vec<CompletionItem>),
    Error(String),
}

type Msg = Option<(u64, Status)>;

/// Reply handle given to a completion provider: answer now with
/// [`Completions::set`], or later (a `Task`'s `on_done`), or from a
/// worker thread through [`Completions::feed`]. Answers to a query the
/// user has typed past are ignored.
#[derive(Clone)]
pub struct Completions {
    id: u64,
    msg: State<Msg>,
    current: Rc<Cell<u64>>,
}

impl Completions {
    pub fn set(&self, items: Vec<CompletionItem>) {
        self.msg.set(Some((self.id, Status::Items(items))));
    }
    /// Shows "Searching…" until the items arrive.
    pub fn loading(&self) {
        self.msg.set(Some((self.id, Status::Loading)));
    }
    pub fn error(&self, message: &str) {
        self.msg.set(Some((self.id, Status::Error(message.into()))));
    }
    /// False once the user typed past this query.
    pub fn is_current(&self) -> bool {
        self.current.get() == self.id
    }
    /// A `Send` handle for a worker thread.
    pub fn feed(&self) -> CompletionFeed {
        CompletionFeed {
            id: self.id,
            feed: self.msg.feed(),
        }
    }
}

/// [`Completions`] for worker threads (`Send`).
pub struct CompletionFeed {
    id: u64,
    feed: StateFeed<Msg>,
}

impl CompletionFeed {
    pub fn set(&self, items: Vec<CompletionItem>) {
        self.feed.set(Some((self.id, Status::Items(items))));
    }
    pub fn error(&self, message: &str) {
        self.feed
            .set(Some((self.id, Status::Error(message.into()))));
    }
}

/// A file or reference shown as a chip above the input.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attachment {
    /// Stable key (a path, an upload ID).
    pub id: String,
    pub name: String,
    /// Dimmed text after the name ("12 KB", "uploading…").
    pub detail: String,
    pub icon: String,
    /// Drawn in the urgent color (a failed upload).
    pub error: bool,
}

impl Attachment {
    pub fn new(id: &str, name: &str) -> Attachment {
        Attachment {
            id: id.into(),
            name: name.into(),
            icon: glyph::FILE.into(),
            ..Default::default()
        }
    }
    /// From a path: the file name, its size, a file or folder icon.
    pub fn file(path: &std::path::Path) -> Attachment {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let meta = std::fs::metadata(path).ok();
        let dir = meta.as_ref().is_some_and(|m| m.is_dir());
        let detail = meta
            .filter(|m| m.is_file())
            .map(|m| size_text(m.len()))
            .unwrap_or_default();
        Attachment {
            id: path.display().to_string(),
            name,
            detail,
            icon: if dir { glyph::FOLDER } else { glyph::FILE }.into(),
            error: false,
        }
    }
    pub fn detail(mut self, text: &str) -> Self {
        self.detail = text.into();
        self
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = glyph.into();
        self
    }
    pub fn error(mut self, on: bool) -> Self {
        self.error = on;
        self
    }
}

/// "512 B", "12 KB", "3.4 MB".
pub fn size_text(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", (b + 512) / 1024),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// What Enter does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendKey {
    /// Enter sends, Shift+Enter is a new line (chat default).
    #[default]
    Enter,
    /// Mod+Enter sends, Enter is a new line (long prompts).
    ModEnter,
}

/// Paths in pasted or dropped text: `file://` URIs (one per line, as file
/// managers put them on the clipboard), or absolute paths when `dropped`
/// (Windows and macOS drops). None when any line is something else.
pub fn parse_files(text: &str, dropped: bool) -> Option<Vec<PathBuf>> {
    let mut out = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if let Some(rest) = line.strip_prefix("file://") {
            // file://host/path → /path
            let path = &rest[rest.find('/')?..];
            let mut bytes = Vec::with_capacity(path.len());
            let b = path.as_bytes();
            let mut i = 0;
            while i < b.len() {
                let hex = |c: u8| (c as char).to_digit(16);
                if b[i] == b'%' && i + 2 < b.len() {
                    if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                        bytes.push((h * 16 + l) as u8);
                        i += 3;
                        continue;
                    }
                }
                bytes.push(b[i]);
                i += 1;
            }
            let mut p = String::from_utf8_lossy(&bytes).into_owned();
            // file:///C:/x → C:/x
            let pb = p.as_bytes();
            if pb.len() >= 3 && pb[0] == b'/' && pb[1].is_ascii_alphabetic() && pb[2] == b':' {
                p.remove(0);
            }
            out.push(PathBuf::from(p));
        } else if dropped && (std::path::Path::new(line).is_absolute() || line.starts_with('/')) {
            out.push(PathBuf::from(line));
        } else {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The completion query at the caret, if any.
pub(crate) fn find_query(
    text: &str,
    caret: usize,
    chips: &[super::Chip],
    triggers: &[(char, Trigger)],
) -> Option<Query> {
    let line_start = text[..caret].rfind('\n').map_or(0, |i| i + 1);
    let chip_end = chips
        .iter()
        .map(|c| c.range.end)
        .filter(|e| *e <= caret && *e >= line_start)
        .max();
    let word_start = text[line_start..caret]
        .rfind(char::is_whitespace)
        .map_or(line_start, |i| {
            let at = line_start + i;
            at + text[at..].chars().next().map_or(1, char::len_utf8)
        });
    let start = word_start.max(chip_end.unwrap_or(0));
    let token = &text[start..caret];
    let trigger = token.chars().next()?;
    let (_, rule) = triggers.iter().find(|(c, _)| *c == trigger)?;
    if chip_end == Some(start) && start > word_start {
        return None; // glued to a chip
    }
    if *rule == Trigger::LineStart && start != line_start {
        return None;
    }
    Some(Query {
        trigger,
        text: token[trigger.len_utf8()..].to_string(),
        range: start..caret,
    })
}

type Provider = Rc<RefCell<Box<dyn FnMut(&Query, Completions)>>>;

/// [`Composer::completion_state`]: labels, highlighted index, card rect.
pub type CompletionState = (Vec<String>, usize, (i32, i32, i32, i32));

#[derive(Default)]
struct Popup {
    query: Option<Query>,
    status: Option<Status>,
    current: usize,
    top: usize,
    /// (revision, caret) where Esc closed it: stays closed until that changes.
    dismissed: Option<(u64, usize)>,
    layer: Option<Widget>,
    rect: (i32, i32, i32, i32),
}

const COMPLETION_KEY: u64 = 0x00c0_0000_0000;
const MAX_ROWS: usize = 8;

struct Data {
    triggers: RefCell<Vec<(char, Trigger, Provider)>>,
    popup: RefCell<Popup>,
    msg: State<Msg>,
    current: Rc<Cell<u64>>,
    next_id: Cell<u64>,
    submit: Hook<String>,
    alt_submit: Hook<String>,
    stop: Hook<()>,
    files: Hook<Vec<PathBuf>>,
    remove: Hook<Attachment>,
    open: Hook<Attachment>,
    send_key: Cell<SendKey>,
    clear_on_submit: Cell<bool>,
    busy: State<bool>,
    custom_placeholder: Cell<bool>,
    attachments: ListModel<String, Attachment>,
    dnd: Cell<bool>,
    laying_out: Cell<bool>,
    history: RefCell<Option<State<Vec<String>>>>,
    hist: RefCell<PromptHistory>,
    /// True while the composer itself replaces the text (history).
    setting: Cell<bool>,
    expanded: State<bool>,
    pick: Hook<(), Vec<PathBuf>>,
    attach: RefCell<Option<Button>>,
    /// `page_keys_to`: the transcript that takes page keys while the text fits.
    pages: RefCell<Option<crate::text::RichText>>,
}

/// The row under the input: `lead` (flexible) holds the attach button
/// and the app's widgets at its start, `end` (fitted) the app's end
/// widgets. Children are only ever appended or inserted: moving a child
/// of a `Flex` drops its fixed size.
struct Bottom {
    row: Flex,
    lead: Flex,
    end: Flex,
    used: bool,
}

/// Chat input (the most used control of an agent client): a [`TextArea`]
/// that grows from 1 to 8 lines, a send button that turns into a stop
/// button while `busy`, attachment chips above, completions for trigger
/// characters (`@files`, `$skills`, `/commands`) in a list at the caret,
/// and file drops / pastes reported with [`Composer::on_files`].
///
/// Keys: Enter sends (Shift+Enter new line; [`SendKey::ModEnter`] or the
/// expanded editor, [`Composer::expanded`], swap them), Mod+Enter always
/// sends, Mod+Shift+Enter is the alternate send when set; with
/// [`Composer::history`], Up on the first line and Down on the last one
/// walk the prompts sent before; with the completion list open Up/Down
/// move, Enter or Tab accept, Esc closes it (a second Esc goes on to the
/// app); Mod+Shift+A attaches files ([`Composer::attach_button`]).
/// Everything else is [`TextArea`]. Draft persistence is the app's: bind
/// a State.
#[derive(Clone)]
pub struct Composer {
    inner: fltk::group::Group,
    area: TextArea,
    bar: Widget,
    send: Button,
    /// Shown instead of `send` while there is nothing to send (a disabled
    /// primary button still paints the accent fill).
    idle: Button,
    stop: Button,
    toolbar: Rc<RefCell<Option<Bottom>>>,
    d: Rc<Data>,
}
fltk::widget_extends!(Composer, fltk::group::Group, inner);

impl Default for Composer {
    fn default() -> Self {
        Self::new()
    }
}

impl Composer {
    pub fn new() -> Self {
        let mut inner = fltk::group::Group::default().with_size(400, 40);
        inner.set_frame(FrameType::NoBox);
        let bar = attachment_bar_widget();
        let area = TextArea::new().lines(1, 8);
        let send = Button::icon(SEND, "").primary();
        let mut idle = Button::icon(SEND, "").bordered();
        idle.deactivate();
        let stop = Button::icon(glyph::STOP, "").danger().bordered();
        inner.end();
        let d = Rc::new(Data {
            triggers: RefCell::new(Vec::new()),
            popup: RefCell::new(Popup::default()),
            msg: State::new(None),
            current: Rc::new(Cell::new(0)),
            next_id: Cell::new(1),
            submit: Hook::default(),
            alt_submit: Hook::default(),
            stop: Hook::default(),
            files: Hook::default(),
            remove: Hook::default(),
            open: Hook::default(),
            send_key: Cell::new(SendKey::Enter),
            clear_on_submit: Cell::new(true),
            busy: State::new(false),
            custom_placeholder: Cell::new(false),
            attachments: ListModel::new(|a: &Attachment| a.id.clone()),
            dnd: Cell::new(false),
            laying_out: Cell::new(false),
            history: RefCell::new(None),
            hist: RefCell::new(PromptHistory::new()),
            setting: Cell::new(false),
            expanded: State::new(false),
            pick: Hook::default(),
            attach: RefCell::new(None),
            pages: RefCell::new(None),
        });
        let me = Composer {
            inner,
            area,
            bar,
            send,
            idle,
            stop,
            toolbar: Rc::default(),
            d,
        };
        me.wire();
        me.layout();
        me
    }

    fn wire(&self) {
        // Placeholder and button names follow the language.
        let me = self.clone();
        crate::i18n::bind(&self.inner, move || {
            if !me.d.custom_placeholder.get() {
                me.area.st.borrow_mut().placeholder = crate::tr!("Write a message…");
                me.area.clone().redraw();
            }
            // The send button follows the reading direction.
            me.layout();
        });
        let me = self.clone();
        crate::i18n::bind(&*self.send, move || me.label_buttons());
        let me = self.clone();
        crate::state::bind_widget(&self.inner, &self.d.expanded, false, move |_| {
            me.label_buttons()
        });
        // An edit by the user leaves the prompt history.
        let me = self.clone();
        let _ = self.area.clone().on_change(move |_| {
            if !me.d.setting.get() {
                me.d.hist.borrow_mut().reset();
            }
        });
        let me = self.clone();
        let send = self.send.clone().on_click(move || me.submit(false));
        let me = self.clone();
        let _ = self.stop.clone().on_click(move || {
            me.d.stop.call(());
        });
        let _ = send;
        // Area hooks.
        let me = self.clone();
        self.area.cbs.height.set(move |_| me.layout());
        let me = self.clone();
        self.area.cbs.caret.set(move |_| {
            me.refresh_buttons();
            me.refresh_completion();
        });
        let me = self.clone();
        self.area.cbs.focus.set(move |on| {
            if on {
                me.refresh_completion();
            } else {
                me.close_popup();
            }
        });
        let me = self.clone();
        self.area.cbs.pre_key.set(move |(k, m)| me.key(k, m));
        let me = self.clone();
        self.area
            .cbs
            .paste
            .set(move |(text, dropped)| me.pasted(&text, dropped));
        let me = self.clone();
        *self.area.cbs.a11y_extra.borrow_mut() = Some(Rc::new(move |n| me.describe_popup(n)));
        let me = self.clone();
        self.area.cbs.a11y_action.set(move |r| me.popup_action(&r));
        // Replies from providers.
        let me = self.clone();
        crate::state::bind_widget(&self.inner, &self.d.msg, false, move |m| {
            me.on_reply(m.clone())
        });
        // Busy and attachments.
        let me = self.clone();
        crate::state::bind_widget(&self.inner, &self.d.busy, false, move |_| {
            me.refresh_buttons()
        });
        let me = self.clone();
        let sub = self.d.attachments.subscribe(move |_, _| {
            me.bar.clone().redraw();
            me.refresh_buttons();
            me.layout();
        });
        // SAFETY: a live widget; the copy only tracks its deletion.
        let guard = unsafe { Widget::from_widget_ptr(self.inner.as_widget_ptr()) };
        crate::state::tie(guard, sub, Box::new(self.d.attachments.clone()));
        attach_bar(&self.bar, self);
        // Layout on resize; drops anywhere on the composer.
        let me = self.clone();
        self.inner
            .clone()
            .resize_callback(move |_, _, _, _, _| me.layout());
        let me = self.clone();
        self.inner.clone().handle(move |_, ev| me.group_event(ev));
        self.refresh_buttons();
    }

    // ------------------------------------------------------------ builder

    /// The input's placeholder (default "Write a message…", translated).
    pub fn placeholder(self, text: &str) -> Self {
        self.d.custom_placeholder.set(true);
        self.area.st.borrow_mut().placeholder = text.into();
        self
    }

    /// Placeholder from a State (`&i18n::text("Ask anything, @files or /commands…")`).
    pub fn bind_placeholder(self, s: &State<String>) -> Self {
        self.d.custom_placeholder.set(true);
        let _ = self.area.clone().bind_placeholder(s);
        self
    }

    /// Two-way binding of the draft text (the app persists drafts).
    pub fn bind(self, s: &State<String>) -> Self {
        let _ = self.area.clone().bind(s);
        self
    }

    /// While true the send button becomes a stop button (when there is no
    /// draft to send) that runs [`Composer::on_stop`].
    pub fn busy(self, busy: &State<bool>) -> Self {
        let me = self.clone();
        crate::state::bind_widget(&self.inner, busy, true, move |b| me.d.busy.set(*b));
        self
    }

    /// Enter sends (default) or Mod+Enter sends.
    pub fn send_key(self, k: SendKey) -> Self {
        self.d.send_key.set(k);
        self.label_buttons();
        self
    }

    /// The expanded editor (o3's full-height Edit mode) while `on` is
    /// true: Enter is a new line and Mod+Enter sends, whatever
    /// [`Composer::send_key`] says; the send button's tooltip shows the
    /// key that sends. The app gives the editor its room (`lines`, layout).
    pub fn expanded(self, on: &State<bool>) -> Self {
        let me = self.clone();
        crate::state::bind_widget(&self.inner, on, true, move |v| me.d.expanded.set(*v));
        self
    }

    /// True while the expanded editor's keys apply.
    pub fn is_expanded(&self) -> bool {
        self.d.expanded.get()
    }

    /// The key that sends now (expanded: Mod+Enter).
    pub fn effective_send_key(&self) -> SendKey {
        if self.d.expanded.get() {
            SendKey::ModEnter
        } else {
            self.d.send_key.get()
        }
    }

    /// Prompts sent before, newest first (the app keeps the list, e.g.
    /// the user messages of the thread). Up on the input's first line
    /// shows the previous one (the draft being written is kept), Down on
    /// the last line the next one, and past the newest the draft comes
    /// back. Typing, sending or a new list go back to the draft's place.
    pub fn history(self, list: &State<Vec<String>>) -> Self {
        *self.d.history.borrow_mut() = Some(list.clone());
        let me = self.clone();
        crate::state::bind_widget(&self.inner, list, false, move |_| {
            me.d.hist.borrow_mut().reset()
        });
        self
    }

    /// True while the input shows a prompt from [`Composer::history`].
    pub fn browsing_history(&self) -> bool {
        self.d.hist.borrow().browsing()
    }

    /// One history step, as Up (`up`) or Down would take it (without the
    /// caret checks); false when it did nothing.
    pub fn history_step(&self, up: bool) -> bool {
        let Some(list) = self.d.history.borrow().clone() else {
            return false;
        };
        let draft = self.area.value();
        let next = list.with(|l| self.d.hist.borrow_mut().step(l, up, &draft));
        let Some(text) = next else { return false };
        self.d.setting.set(true);
        self.area.set_value(&text);
        self.d.setting.set(false);
        // o3: an older prompt starts at the top, a newer one at the end.
        self.area.set_caret(if up { 0 } else { text.len() });
        true
    }

    /// A paperclip button at the start of the bottom row (also
    /// Mod+Shift+A in the input) that opens the system's file chooser;
    /// the picked paths go to [`Composer::on_files`], like drops.
    pub fn attach_button(self) -> Self {
        if self.d.attach.borrow().is_some() {
            return self;
        }
        let me = self.clone();
        let b = Button::icon(ATTACH, "").on_click(move || me.pick_files());
        let b2 = b.clone();
        crate::i18n::bind(&*b, move || {
            let tip = crate::tr!("Attach files");
            let mut b = b2.clone();
            b.set_tooltip(&format!("{tip} · {}", crate::keys::label("Mod+Shift+A")));
            a11y::set_name(&*b, &tip);
        });
        let mut row = self.bottom();
        crate::layout::remember(&*b);
        row.insert(&*b, 0);
        row.fit(&*b);
        if let Some(t) = self.toolbar.borrow_mut().as_mut() {
            t.used = true;
        }
        *self.d.attach.borrow_mut() = Some(b);
        self.layout();
        self
    }

    /// The attach button, when [`Composer::attach_button`] added it.
    pub fn attach(&self) -> Option<Button> {
        self.d.attach.borrow().clone()
    }

    /// Replaces the system file chooser of [`Composer::pick_files`]
    /// (a portal, a remote picker, tests): `f()` returns the picked paths
    /// (empty: cancelled).
    pub fn on_pick_files(self, mut f: impl FnMut() -> Vec<PathBuf> + 'static) -> Self {
        self.d.pick.set(move |_| f());
        self
    }

    /// Opens the file chooser (several files; [`Composer::on_pick_files`]
    /// replaces it) and gives the picked paths to [`Composer::on_files`].
    pub fn pick_files(&self) {
        if !self.d.files.is_set() {
            return;
        }
        let paths = match self.d.pick.call(()) {
            Some(p) => p,
            None => {
                // The portal (or the native chooser) answers later on the
                // UI thread; the composer keeps working meanwhile.
                let me = self.clone();
                crate::dialog::Request::open(crate::tr!("Attach files"))
                    .multiple(true)
                    .run_async(move |r| {
                        match r {
                            Ok(paths) => {
                                me.d.files.call(paths);
                            }
                            Err(crate::dialog::Error::Cancelled) => {}
                            Err(e) => eprintln!("fltk-omarchy: {e}"),
                        }
                        let _ = me.area.clone().take_focus();
                    });
                return;
            }
        };
        if !paths.is_empty() {
            self.d.files.call(paths);
        }
        let _ = self.area.clone().take_focus();
    }


    /// Clear the text and the attachments after `on_submit` (default true).
    pub fn clear_on_submit(self, on: bool) -> Self {
        self.d.clear_on_submit.set(on);
        self
    }

    /// Text grows from `min` to `max` lines (default 1..8).
    pub fn lines(self, min: usize, max: usize) -> Self {
        let _ = self.area.clone().lines(min, max);
        self.layout();
        self
    }

    /// Enter (or the send button) with text or attachments: `f(text)`.
    /// Chips are in the text as their own text; see [`Composer::expanded_text`].
    pub fn on_submit(self, f: impl FnMut(String) + 'static) -> Self {
        self.d.submit.set(f);
        self
    }

    /// While the text fits (no scrolling inside the composer), PageUp /
    /// PageDown scroll `view` by most of a screen and Mod+Home / Mod+End go
    /// to its start / end (following it), without leaving the composer.
    pub fn page_keys_to(self, view: &crate::text::RichText) -> Self {
        *self.d.pages.borrow_mut() = Some(view.clone());
        self
    }

    /// Mod+Shift+Enter (e.g. "send and queue" / "steer").
    pub fn on_alternate_submit(self, f: impl FnMut(String) + 'static) -> Self {
        self.d.alt_submit.set(f);
        self
    }

    /// The stop button (shown while busy).
    pub fn on_stop(self, mut f: impl FnMut() + 'static) -> Self {
        self.d.stop.set(move |_| f());
        self
    }

    /// Files dropped on the composer or pasted as `file://` URIs. Without
    /// this callback dropped text is inserted as text.
    pub fn on_files(self, f: impl FnMut(Vec<PathBuf>) + 'static) -> Self {
        self.d.files.set(f);
        self
    }

    /// An attachment chip's × (or Delete on it). The chip is removed from
    /// [`Composer::attachments`] before `f` runs.
    pub fn on_remove_attachment(self, f: impl FnMut(Attachment) + 'static) -> Self {
        self.d.remove.set(f);
        self
    }

    /// Click or Enter on an attachment chip (preview).
    pub fn on_open_attachment(self, f: impl FnMut(Attachment) + 'static) -> Self {
        self.d.open.set(f);
        self
    }

    /// Completions for `trigger`: `provider(&query, reply)` answers with
    /// `reply.set(items)` now or later (see [`Completions`]).
    pub fn trigger(
        self,
        trigger: char,
        rule: Trigger,
        provider: impl FnMut(&Query, Completions) + 'static,
    ) -> Self {
        let p: Provider = Rc::new(RefCell::new(Box::new(provider)));
        let mut t = self.d.triggers.borrow_mut();
        t.retain(|(c, _, _)| *c != trigger);
        t.push((trigger, rule, p));
        drop(t);
        self
    }

    /// Sees every key before the composer (history on Up, app keys).
    pub fn on_key(self, f: impl FnMut(Key, Shortcut) -> bool + 'static) -> Self {
        let _ = self.area.clone().on_key(f);
        self
    }

    // ------------------------------------------------------------ access

    /// The editor (value, chips, selection, undo, caret queries).
    pub fn area(&self) -> TextArea {
        self.area.clone()
    }

    /// The attachment chips shown above the input (push, remove, patch).
    pub fn attachments(&self) -> ListModel<String, Attachment> {
        self.d.attachments.clone()
    }

    /// The bottom row under the input for the app's widgets (model,
    /// permission and branch pickers, a dictation level ring): `build`
    /// fills it like any row (`row.fit(&*button)`), at its start, after
    /// the attach button. The send button stays at the input's end. The
    /// row mirrors in a right-to-left language; Tab goes input → send →
    /// the row's widgets in the order they were added.
    pub fn toolbar(self, build: impl FnOnce(&mut Flex)) -> Self {
        let mut t = self.bottom();
        let prev = fltk::group::Group::try_current();
        t.begin();
        build(&mut t);
        t.end();
        if let Some(b) = self.toolbar.borrow_mut().as_mut() {
            b.used = true;
        }
        if let Some(g) = prev {
            g.begin();
        }
        self.layout();
        self
    }

    /// Widgets at the end of the bottom row (a context meter, draft
    /// status, an expand button), before nothing but the row's end:
    /// `build` fills them like [`Composer::toolbar`].
    pub fn toolbar_end(self, build: impl FnOnce(&mut Flex)) -> Self {
        let _ = self.bottom();
        let end = self.toolbar.borrow().as_ref().map(|b| b.end.clone());
        let Some(mut end) = end else { return self };
        let prev = fltk::group::Group::try_current();
        end.begin();
        build(&mut end);
        end.close_fitted();
        if let Some(g) = prev {
            g.begin();
        }
        if let Some(b) = self.toolbar.borrow_mut().as_mut() {
            b.used = true;
        }
        self.layout();
        self
    }

    /// The start of the bottom row (made on first use).
    fn bottom(&self) -> Flex {
        if let Some(b) = self.toolbar.borrow().as_ref() {
            return b.lead.clone();
        }
        let prev = fltk::group::Group::try_current();
        fltk::group::Group::set_current(None::<&fltk::group::Group>);
        let mut row = crate::layout::row();
        let lead = crate::layout::row();
        lead.end();
        let end = crate::layout::row();
        end.end();
        row.fixed(&end, 0);
        row.end();
        self.inner.clone().add(&row);
        if let Some(g) = prev {
            g.begin();
        }
        *self.toolbar.borrow_mut() = Some(Bottom {
            row,
            lead: lead.clone(),
            end,
            used: false,
        });
        lead
    }


    /// Tooltips and names of the send and stop buttons; the send tooltip
    /// shows the key that sends.
    fn label_buttons(&self) {
        let (mut a, mut b, mut c) = (self.send.clone(), self.stop.clone(), self.idle.clone());
        let (send, stop) = (crate::tr!("Send"), crate::tr!("Stop"));
        let key = match self.effective_send_key() {
            SendKey::Enter => "Enter".to_string(),
            SendKey::ModEnter => crate::keys::label("Mod+Enter"),
        };
        let tip = format!("{send} · {key}");
        a.set_tooltip(&tip);
        c.set_tooltip(&tip);
        b.set_tooltip(&stop);
        a11y::set_name(&*a, &send);
        a11y::set_name(&*c, &send);
        a11y::set_name(&*b, &stop);
        a11y::set_description(&*a, &key);
        a11y::set_description(&*c, &key);
    }

    pub fn value(&self) -> String {
        self.area.value()
    }

    pub fn set_value(&self, text: &str) {
        self.area.set_value(text);
    }

    /// The text with every chip replaced by `f(chip_text, data)`.
    pub fn expanded_text(&self, f: impl FnMut(&str, &str) -> String) -> String {
        self.area.expanded_text(f)
    }

    pub fn is_busy(&self) -> bool {
        self.d.busy.get()
    }

    /// True while the stop button shows instead of send.
    pub fn shows_stop(&self) -> bool {
        self.stop.visible()
    }

    pub fn send_button(&self) -> Button {
        self.send.clone()
    }

    pub fn stop_button(&self) -> Button {
        self.stop.clone()
    }

    /// True while the completion list is open.
    pub fn completion_open(&self) -> bool {
        self.d
            .popup
            .borrow()
            .layer
            .as_ref()
            .is_some_and(|l| !l.was_deleted())
    }

    /// The open list: (labels, highlighted index, rect in window coordinates).
    pub fn completion_state(&self) -> Option<CompletionState> {
        let p = self.d.popup.borrow();
        p.layer.as_ref()?;
        let labels = match &p.status {
            Some(Status::Items(v)) => v.iter().map(|i| i.label.clone()).collect(),
            _ => Vec::new(),
        };
        Some((labels, p.current, p.rect))
    }

    /// Sends now (as Enter would).
    pub fn submit(&self, alternate: bool) {
        if !self.d.submit.is_set() && !(alternate && self.d.alt_submit.is_set()) {
            return;
        }
        self.close_popup();
        let text = self.area.value();
        if text.trim().is_empty() && self.d.attachments.is_empty() {
            return;
        }
        if alternate && self.d.alt_submit.is_set() {
            self.d.alt_submit.call(text);
        } else {
            self.d.submit.call(text);
        }
        self.d.hist.borrow_mut().reset();
        if self.d.clear_on_submit.get() {
            self.d.setting.set(true);
            self.area.set_value("");
            self.d.setting.set(false);
            self.d.attachments.clear();
        }

    }

    // ------------------------------------------------------------ layout

    fn has_content(&self) -> bool {
        !self.area.with_buffer(|b| b.text().trim().is_empty()) || !self.d.attachments.is_empty()
    }

    fn refresh_buttons(&self) {
        let content = self.has_content();
        let stop_mode = self.d.busy.get() && !content;
        let (mut send, mut idle, mut stop) =
            (self.send.clone(), self.idle.clone(), self.stop.clone());
        let had_focus = [&send, &idle, &stop]
            .iter()
            .any(|b| crate::focus::is_focused(&***b));
        let show = if stop_mode {
            2
        } else if content {
            0
        } else {
            1
        };
        for (i, b) in [&mut send, &mut idle, &mut stop].into_iter().enumerate() {
            if i == show {
                if !b.visible() {
                    b.show();
                    if had_focus {
                        let _ = b.take_focus();
                    }
                }
            } else if b.visible() {
                b.hide();
            }
        }
        if had_focus && show == 1 {
            // The disabled button cannot keep focus: back to the input.
            let _ = self.area.clone().take_focus();
        }
    }

    /// Positions the children and refits the composer to its natural height.
    pub fn layout(&self) {
        if self.d.laying_out.replace(true) {
            return;
        }
        let (x, y, w) = (self.inner.x(), self.inner.y(), self.inner.w());
        let (gap, bh) = theme::with(|t| (t.spacing.control_gap, t.spacing.control_height));
        let mut cy = y;
        let mut bar = self.bar.clone();
        if self.d.attachments.is_empty() {
            bar.hide();
        } else {
            bar.show();
            let h = bar_height(&self.d.attachments, w);
            bar.resize(x, cy, w, h);
            cy += h + gap;
        }
        let aw = (w - bh - gap).max(40);
        // Right-to-left: the send button goes to the reading end (left).
        let rtl = crate::i18n::is_rtl();
        let (ax, bx) = if rtl {
            (x + bh + gap, x)
        } else {
            (x, x + aw + gap)
        };
        let mut area = self.area.clone();
        let h0 = area.h();
        area.resize(ax, cy, aw, h0);
        let ah = area.natural_h();
        area.resize(ax, cy, aw, ah);
        let by = cy + ah - bh;
        self.send.clone().resize(bx, by, bh, bh);
        self.idle.clone().resize(bx, by, bh, bh);
        self.stop.clone().resize(bx, by, bh, bh);
        cy += ah;
        if let Some((mut tb, used)) = self.toolbar.borrow().as_ref().map(|b| (b.row.clone(), b.used)) {
            if used {
                tb.show();
                cy += gap;
                tb.resize(x, cy, w, bh);
                tb.layout();                cy += bh;
            } else {
                tb.hide();
            }
        }
        let total = cy - y;
        self.d.laying_out.set(false);
        if total != self.inner.h() {
            let mut g = self.inner.clone();
            if !crate::layout::refit(&g, (w, total)) {
                g.set_size(w, total);
                if let Some(mut p) = g.parent() {
                    p.redraw();
                }
            }
        }
        self.inner.clone().redraw();
        if self.completion_open() {
            self.show_popup();
        }
    }

    /// Natural size (width as created, height of the content).
    pub fn preferred_size(&self) -> (i32, i32) {
        (self.inner.w(), self.inner.h())
    }

    // ------------------------------------------------------------ events

    fn key(&self, k: Key, m: Shortcut) -> bool {
        let open = self.completion_open();
        let plain = m.is_empty();
        if open && plain {
            let n = self.popup_len();
            match k {
                Key::Up | Key::Down | Key::PageUp | Key::PageDown => {
                    if n > 0 {
                        let mut p = self.d.popup.borrow_mut();
                        let c = p.current as isize;
                        let step = match k {
                            Key::Up => -1,
                            Key::Down => 1,
                            Key::PageUp => -(MAX_ROWS as isize),
                            _ => MAX_ROWS as isize,
                        };
                        p.current = if matches!(k, Key::Up | Key::Down) {
                            (c + step).rem_euclid(n as isize) as usize
                        } else {
                            (c + step).clamp(0, n as isize - 1) as usize
                        };
                        reveal(&mut p);
                    }
                    self.redraw_popup();
                    return true;
                }
                Key::Enter | Key::KPEnter | Key::Tab => {
                    if n > 0 {
                        let i = self.d.popup.borrow().current;
                        self.accept(i);
                        return true;
                    }
                    self.close_popup();
                    if k == Key::Tab {
                        return true;
                    }
                }
                Key::Escape => {
                    self.dismiss();
                    return true;
                }
                _ => {}
            }
        }
        if plain && matches!(k, Key::Up | Key::Down) && self.history_key(k) {
            return true;
        }
        // While the text fits, page keys scroll the transcript beside us
        // (Rune: a composer that fits passes them to the message list).
        let mod_only = crate::keys::is_primary(m) && (m & !crate::keys::PRIMARY).is_empty();
        if (plain && matches!(k, Key::PageUp | Key::PageDown)) || (mod_only && matches!(k, Key::Home | Key::End)) {
            let view = self.d.pages.borrow().clone();
            if let Some(v) = view.filter(|_| self.area.fits()) {
                let page = (v.h() as i64 * 4 / 5).max(1);
                match k {
                    Key::PageUp => v.scroll_to(v.scroll_offset() - page),
                    Key::PageDown => v.scroll_to(v.scroll_offset() + page),
                    Key::Home => v.scroll_to(0),
                    _ => v.follow_end(),
                }
                return true;
            }
        }
        if crate::keys::is_primary(m)
            && m.contains(Shortcut::Shift)
            && (k == Key::from_char('a') || k == Key::from_char('A'))
            && self.d.attach.borrow().is_some()
        {
            self.pick_files();
            return true;
        }
        if matches!(k, Key::Enter | Key::KPEnter) {
            if self.area.is_composing() {
                return false; // the input method commits first
            }
            let primary = crate::keys::is_primary(m);
            let shift = m.contains(Shortcut::Shift);
            let others = m.intersects(
                Shortcut::Alt
                    | if crate::keys::PRIMARY == Shortcut::Ctrl {
                        Shortcut::Meta
                    } else {
                        Shortcut::Ctrl
                    },
            );
            if others {
                return false;
            }
            if !self.d.submit.is_set() {
                return false; // nothing to send to: Enter is a new line
            }
            if primary && shift {
                self.submit(true);
                return true;
            }
            let sends = primary || (!shift && self.effective_send_key() == SendKey::Enter);
            if sends {
                self.submit(false);
                return true;
            }
        }
        false
    }

    /// Up/Down through the prompt history (o3: the caret on the first /
    /// last line, nothing selected, no input method composing).
    fn history_key(&self, k: Key) -> bool {
        if self.d.history.borrow().is_none() || self.area.is_composing() {
            return false;
        }
        if !self.area.selection().is_empty() {
            return false;
        }
        let up = k == Key::Up;
        let edge = if up {
            self.area.caret_on_first_line()
        } else {
            self.area.caret_on_last_line()
        };
        edge && self.history_step(up)
    }

    fn pasted(&self, text: &str, dropped: bool) -> bool {
        if !self.d.files.is_set() {
            return false;
        }
        match parse_files(text, dropped) {
            Some(paths) => {
                self.d.files.call(paths);
                true
            }
            None => false,
        }
    }

    fn group_event(&self, ev: Event) -> bool {
        match ev {
            Event::DndEnter | Event::DndDrag | Event::DndLeave | Event::DndRelease => {
                if app::event_inside_widget(&*self.area) {
                    return false;
                }
                if !self.d.files.is_set() {
                    return false;
                }
                if ev == Event::DndRelease {
                    self.d.dnd.set(true);
                }
                true
            }
            Event::Paste if self.d.dnd.replace(false) => {
                let text = app::event_text();
                if !self.pasted(&text, true) {
                    self.area.insert(&text.replace("\r\n", "\n"));
                }
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------ completions

    fn popup_len(&self) -> usize {
        match &self.d.popup.borrow().status {
            Some(Status::Items(v)) => v.len(),
            _ => 0,
        }
    }

    fn refresh_completion(&self) {
        let triggers: Vec<(char, Trigger)> = self
            .d
            .triggers
            .borrow()
            .iter()
            .map(|(c, r, _)| (*c, *r))
            .collect();
        if triggers.is_empty() {
            return;
        }
        let focused = crate::focus::is_focused(&*self.area);
        let (q, key) = self.area.with_buffer(|b| {
            let q = if b.has_selection() || b.preedit().is_some() {
                None
            } else {
                find_query(b.text(), b.caret(), b.chips(), &triggers)
            };
            (q, (b.revision(), b.caret()))
        });
        let dismissed = self.d.popup.borrow().dismissed == Some(key);
        let Some(q) = q.filter(|_| focused && !dismissed) else {
            self.close_popup();
            self.d.popup.borrow_mut().query = None;
            return;
        };
        {
            let p = self.d.popup.borrow();
            if p.query.as_ref() == Some(&q) {
                drop(p);
                if self.completion_open() {
                    self.show_popup();
                }
                return;
            }
        }
        let id = self.d.next_id.get();
        self.d.next_id.set(id + 1);
        self.d.current.set(id);
        {
            let mut p = self.d.popup.borrow_mut();
            let same_trigger = p
                .query
                .as_ref()
                .is_some_and(|o| o.trigger == q.trigger && o.range.start == q.range.start);
            p.query = Some(q.clone());
            p.dismissed = None;
            if !same_trigger {
                p.status = None;
            }
        }
        let provider = self
            .d
            .triggers
            .borrow()
            .iter()
            .find(|(c, _, _)| *c == q.trigger)
            .map(|(_, _, p)| p.clone());
        if let Some(p) = provider {
            let reply = Completions {
                id,
                msg: self.d.msg.clone(),
                current: self.d.current.clone(),
            };
            (p.borrow_mut())(&q, reply);
        }
    }

    fn on_reply(&self, m: Msg) {
        let Some((id, status)) = m else { return };
        if id != self.d.current.get() || self.d.popup.borrow().query.is_none() {
            return;
        }
        {
            let mut p = self.d.popup.borrow_mut();
            let keep = matches!((&p.status, &status), (Some(Status::Items(a)), Status::Items(b)) if a.get(p.current) == b.get(p.current));
            p.status = Some(status);
            if !keep {
                p.current = 0;
                p.top = 0;
            }
        }
        if crate::focus::is_focused(&*self.area) {
            self.show_popup();
        }
    }

    /// Esc: closes and stays closed until the text or the caret changes.
    fn dismiss(&self) {
        let key = self.area.with_buffer(|b| (b.revision(), b.caret()));
        self.close_popup();
        self.d.popup.borrow_mut().dismissed = Some(key);
    }

    fn close_popup(&self) {
        let layer = self.d.popup.borrow_mut().layer.take();
        if let Some(l) = layer {
            if !l.was_deleted() {
                close_layer(&l);
            }
            self.area.clone().redraw();
        }
    }

    fn accept(&self, i: usize) {
        let (q, item) = {
            let p = self.d.popup.borrow();
            let item = match &p.status {
                Some(Status::Items(v)) => v.get(i).cloned(),
                _ => None,
            };
            (p.query.clone(), item)
        };
        let (Some(q), Some(item)) = (q, item) else {
            return;
        };
        self.close_popup();
        let exact = !item.insert.is_empty();
        let text = if exact {
            item.insert.clone()
        } else {
            format!("{}{}", q.trigger, item.label)
        };
        let area = &self.area;
        match &item.chip {
            Some(data) => {
                area.replace_with_chip(q.range.clone(), text.trim_end(), data);
                let next_blank = area.with_buffer(|b| {
                    b.text()[b.caret()..]
                        .chars()
                        .next()
                        .map_or(true, char::is_whitespace)
                });
                if !next_blank || !exact {
                    if area.with_buffer(|b| !b.text()[b.caret()..].starts_with(' ')) {
                        area.insert(" ");
                    } else {
                        let c = area.caret();
                        area.set_caret(c + 1);
                    }
                }
            }
            None => {
                let t = if exact { text } else { format!("{text} ") };
                area.replace(q.range.clone(), &t);
            }
        }
        let key = area.with_buffer(|b| (b.revision(), b.caret()));
        let mut p = self.d.popup.borrow_mut();
        p.dismissed = Some(key);
        p.query = None;
    }

    fn geometry(&self) -> Option<(i32, i32, i32, i32)> {
        let win = self.area.top_window()?;
        let p = self.d.popup.borrow();
        let q = p.query.as_ref()?;
        let rows = match &p.status {
            Some(Status::Items(v)) if !v.is_empty() => v.len().min(MAX_ROWS),
            Some(_) => 1,
            None => return None,
        };
        let (row, pad) = theme::with(|t| (t.spacing.popup_row_height, 2 + t.spacing.xxs));
        let h = rows as i32 * row + 2 * pad;
        let w = theme::with(|t| {
            let mut w = 240;
            if let Some(Status::Items(v)) = &p.status {
                for it in v.iter().take(200) {
                    let lw = crate::widgets::measure_offscreen(&it.label, t.font.body);
                    let dw = if it.detail.is_empty() {
                        0
                    } else {
                        crate::widgets::measure_offscreen(&it.detail, t.font.caption).min(220)
                            + t.spacing.xl
                    };
                    w = w.max(
                        lw + dw
                            + 2 * t.spacing.control_padding_x
                            + if it.icon.is_empty() {
                                0
                            } else {
                                t.font.icon + t.spacing.lg
                            }
                            + 12,
                    );
                }
            }
            w.min(480)
        });
        let w = w.min(win.w() - 8).max(60);
        let (cx, cy, _, ch) = self.area.caret_rect_at(Some(q.range.start));
        let below = cy + ch + 4;
        let above = cy - 4 - h;
        let y = if below + h <= win.h() - 4 {
            below
        } else if above >= 4 {
            above
        } else if win.h() - below > cy {
            below
        } else {
            4
        };
        let pad_x = theme::with(|t| t.spacing.control_padding_x);
        let x = (cx - pad_x).clamp(4, (win.w() - w - 4).max(4));
        Some((x, y, w, h.min(win.h() - 8)))
    }

    fn show_popup(&self) {
        let Some(rect) = self.geometry() else {
            self.close_popup();
            return;
        };
        let existing = self
            .d
            .popup
            .borrow()
            .layer
            .clone()
            .filter(|l| !l.was_deleted());
        let mut layer = match existing {
            Some(l) => l,
            None => {
                let Some(l) = open_layer(&*self.area, Some(rect), false) else {
                    return;
                };
                self.install_layer(&l);
                l
            }
        };
        let old = self.d.popup.borrow().rect;
        {
            let mut p = self.d.popup.borrow_mut();
            p.rect = rect;
            p.layer = Some(layer.clone());
            reveal(&mut p);
        }
        if old != rect {
            layer.resize(rect.0, rect.1, rect.2, rect.3);
            if let Some(mut w) = layer.top_window() {
                w.redraw();
            }
        }
        layer.redraw();
        self.area.clone().redraw();
    }

    fn redraw_popup(&self) {
        if let Some(mut l) = self.d.popup.borrow().layer.clone() {
            l.redraw();
        }
        self.area.clone().redraw();
    }

    fn row_at(&self, ey: i32) -> Option<usize> {
        let p = self.d.popup.borrow();
        let (_, y, _, h) = p.rect;
        let (row, pad) = theme::with(|t| (t.spacing.popup_row_height, 2 + t.spacing.xxs));
        if ey < y + pad || ey >= y + h - pad {
            return None;
        }
        let i = p.top + ((ey - y - pad) / row) as usize;
        match &p.status {
            Some(Status::Items(v)) if i < v.len() => Some(i),
            _ => None,
        }
    }

    fn install_layer(&self, layer: &Widget) {
        let mut layer = layer.clone();
        let me = self.clone();
        crate::overlay::draw_layer(&mut layer, move |l| {
            if me.area.was_deleted() {
                // The composer went away with its list open.
                let l2 = l.clone();
                crate::timer::add(0.0, move |_| {
                    if !l2.was_deleted() {
                        close_layer(&l2);
                    }
                });
                return;
            }
            let p = me.d.popup.borrow();
            let (x, y, w, h) = p.rect;
            theme::with(|t| {
                card(
                    t,
                    x,
                    y,
                    w,
                    h,
                    &t.popups,
                    t.controls.normal.border_width.max(1),
                );
                let fill = t.popups.fill(bg(t));
                let fg = t.popups.text;
                let dim = fg.darker(1.5);
                let (row, pad) = (t.spacing.popup_row_height, 2 + t.spacing.xxs);
                let padx = t.spacing.control_padding_x;
                fltk::draw::push_clip(x + 1, y + pad, w - 2, h - 2 * pad);
                match &p.status {
                    Some(Status::Items(items)) if !items.is_empty() => {
                        let icons = items.iter().any(|i| !i.icon.is_empty());
                        let cell = if icons { t.font.icon + t.spacing.lg } else { 0 };
                        for (k, it) in items.iter().enumerate().skip(p.top).take(MAX_ROWS) {
                            let ry = y + pad + (k - p.top) as i32 * row;
                            if k == p.current {
                                paint::fill(
                                    x + 2,
                                    ry,
                                    w - 4,
                                    row,
                                    t.radius,
                                    t.fill(&t.controls.hover, fill),
                                );
                            }
                            if !it.icon.is_empty() {
                                paint::glyph(
                                    &it.icon,
                                    x + padx,
                                    ry,
                                    t.font.icon,
                                    row,
                                    t.font.icon,
                                    t.palette.accent,
                                );
                            }
                            let tx = x + padx + cell;
                            let avail = x + w - padx - tx;
                            let lw = paint::measure(&it.label, t.font.body, false).min(avail);
                            paint::text_elided(
                                &it.label,
                                tx,
                                ry,
                                avail,
                                row,
                                t.font.body,
                                false,
                                fg,
                                Align::Left,
                            );
                            if !it.detail.is_empty() && avail - lw > 40 {
                                let dx = tx + lw + t.spacing.lg;
                                paint::text_elided(
                                    &it.detail,
                                    dx,
                                    ry,
                                    x + w - padx - dx,
                                    row,
                                    t.font.caption,
                                    false,
                                    dim,
                                    Align::Left,
                                );
                            }
                        }
                        let n = items.len();
                        if n > MAX_ROWS {
                            let track = h - 2 * pad;
                            let thumb =
                                ((MAX_ROWS as f64 / n as f64) * track as f64).max(12.0) as i32;
                            let ty = y
                                + pad
                                + ((p.top as f64 / (n - MAX_ROWS) as f64) * (track - thumb) as f64)
                                    as i32;
                            paint::fill(x + w - 6, ty, 4, thumb, 2, fg.over(fill, 0.3));
                        }
                    }
                    s => {
                        let msg = match s {
                            Some(Status::Error(e)) => e.clone(),
                            Some(Status::Loading) => crate::tr!("Searching…"),
                            _ => crate::tr!("No results"),
                        };
                        let color = if matches!(s, Some(Status::Error(_))) {
                            t.palette.urgent
                        } else {
                            dim
                        };
                        paint::text_elided(
                            &msg,
                            x + padx,
                            y + pad,
                            w - 2 * padx,
                            row,
                            t.font.caption,
                            false,
                            color,
                            Align::Left,
                        );
                    }
                }
                fltk::draw::pop_clip();
            });
            let _ = l;
        });
        let me = self.clone();
        layer.handle(move |l, ev| match ev {
            Event::Enter => true,
            Event::Move => {
                if let Some(i) = me.row_at(app::event_y()) {
                    let changed = {
                        let mut p = me.d.popup.borrow_mut();
                        std::mem::replace(&mut p.current, i) != i
                    };
                    if changed {
                        l.redraw();
                    }
                }
                true
            }
            Event::Push => {
                if let Some(i) = me.row_at(app::event_y()) {
                    me.accept(i);
                }
                true
            }
            Event::MouseWheel => {
                let n = me.popup_len();
                let mut p = me.d.popup.borrow_mut();
                let top = (p.top as i32 + app::event_dy_value())
                    .clamp(0, n.saturating_sub(MAX_ROWS) as i32) as usize;
                p.top = top;
                p.current = p.current.clamp(top, (top + MAX_ROWS).min(n.max(1)) - 1);
                drop(p);
                l.redraw();
                true
            }
            _ => false,
        });
        a11y::describe(&layer, |n| {
            // Exposed as the list under the input (see describe_popup).
            n.role(Role::Unknown).hidden(true).leaf();
        });
    }

    fn describe_popup(&self, n: &mut a11y::Node) {
        if self.d.triggers.borrow().is_empty() {
            return;
        }
        let open = self.completion_open();
        n.expanded(open);
        if !open {
            return;
        }
        let p = self.d.popup.borrow();
        let (x, y, w, h) = p.rect;
        let items = match &p.status {
            Some(Status::Items(v)) => v.clone(),
            _ => Vec::new(),
        };
        let cur = p.current;
        let (top, row, pad) = (
            p.top,
            theme::with(|t| t.spacing.popup_row_height),
            theme::with(|t| 2 + t.spacing.xxs),
        );
        n.child(COMPLETION_KEY, |list| {
            list.role(Role::ListBox)
                .name(&crate::tr!("Suggestions"))
                .bounds(x, y, w, h);
            for (i, it) in items
                .iter()
                .enumerate()
                .skip(cur.saturating_sub(50))
                .take(101)
            {
                list.child(i as u64, |o| {
                    o.role(Role::ListBoxOption)
                        .name(&it.label)
                        .description(&it.detail)
                        .selected(i == cur)
                        .position(i + 1, items.len())
                        .action(Action::Click);
                    if i >= top && i < top + MAX_ROWS {
                        o.bounds(x + 2, y + pad + (i - top) as i32 * row, w - 4, row);
                    }
                });
            }
        });
        if !items.is_empty() {
            n.active_descendant(&[COMPLETION_KEY, cur as u64]);
        }
    }

    fn popup_action(&self, r: &a11y::Request) -> bool {
        if r.path.first() != Some(&COMPLETION_KEY) {
            return false;
        }
        let Some(i) = r
            .path
            .get(1)
            .map(|k| *k as usize)
            .filter(|i| *i < self.popup_len())
        else {
            return false;
        };
        match r.action {
            Action::Click => self.accept(i),
            Action::Focus | Action::ScrollIntoView => {
                let mut p = self.d.popup.borrow_mut();
                p.current = i;
                reveal(&mut p);
                drop(p);
                self.redraw_popup();
            }
            _ => return false,
        }
        true
    }
}

fn reveal(p: &mut Popup) {
    if p.current < p.top {
        p.top = p.current;
    } else if p.current >= p.top + MAX_ROWS {
        p.top = p.current + 1 - MAX_ROWS;
    }
}

// ------------------------------------------------------------ attachments

fn chip_h() -> i32 {
    theme::with(|t| t.spacing.control_height - 4)
}

/// (x, y, w, close_x) of each chip, relative to the bar.
fn chip_rects(items: &[Attachment], w: i32) -> Vec<(i32, i32, i32, i32)> {
    theme::with(|t| {
        let (h, gap, pad) = (chip_h(), t.spacing.sm, t.spacing.md);
        let close = h - 6;
        let (mut x, mut y) = (0, 0);
        let mut out = Vec::new();
        for a in items {
            let iw = if a.icon.is_empty() {
                0
            } else {
                t.font.icon + t.spacing.sm
            };
            let nw = crate::widgets::measure_offscreen(&a.name, t.font.caption).min(200);
            let dw = if a.detail.is_empty() {
                0
            } else {
                crate::widgets::measure_offscreen(&a.detail, t.font.caption) + t.spacing.sm
            };
            let cw = (pad + iw + nw + dw + t.spacing.sm + close + 3).min(w.max(40));
            if x > 0 && x + cw > w {
                x = 0;
                y += h + gap;
            }
            out.push((x, y, cw, x + cw - close - 3));
            x += cw + gap;
        }
        out
    })
}

fn bar_height(m: &ListModel<String, Attachment>, w: i32) -> i32 {
    let items = m.to_vec();
    chip_rects(&items, w)
        .iter()
        .map(|r| r.1)
        .max()
        .map_or(0, |y| y + chip_h())
}

fn attachment_bar_widget() -> Widget {
    let mut w = Widget::default().with_size(100, 24);
    w.set_frame(FrameType::NoBox);
    w.set_visible_focus();
    w.hide();
    w
}

fn attach_bar(bar: &Widget, c: &Composer) {
    let mut bar = bar.clone();
    let cur = Rc::new(Cell::new(0usize));
    let hover: Rc<Cell<Option<(usize, bool)>>> = Rc::new(Cell::new(None));
    let (m, cr, hv) = (c.d.attachments.clone(), cur.clone(), hover.clone());
    bar.draw(move |b| {
        let items = m.to_vec();
        let rects = chip_rects(&items, b.w());
        let ring = crate::focus::show_ring(b);
        theme::with(|t| {
            let under = t.palette.background;
            let h = chip_h();
            for (i, (a, (rx, ry, rw, cx))) in items.iter().zip(rects).enumerate() {
                let (x, y) = (b.x() + rx, b.y() + ry);
                let hov = hv.get().filter(|(k, _)| *k == i);
                let st = if ring && cr.get() == i {
                    &t.controls.focus
                } else if hov.is_some() {
                    &t.controls.hover
                } else {
                    &t.controls.normal
                };
                let fill = t.fill(st, under);
                let border = if a.error {
                    t.palette.urgent
                } else {
                    t.border(st, under)
                };
                paint::surface(
                    x,
                    y,
                    rw,
                    h,
                    h / 2,
                    Some(fill),
                    Some((border, st.border_width.max(1))),
                );
                let fg = if a.error {
                    t.palette.urgent
                } else {
                    t.palette.foreground
                };
                let mut tx = x + t.spacing.md;
                if !a.icon.is_empty() {
                    paint::glyph(
                        &a.icon,
                        tx,
                        y,
                        t.font.icon,
                        h,
                        t.font.icon,
                        t.palette.accent,
                    );
                    tx += t.font.icon + t.spacing.sm;
                }
                let close_x = b.x() + cx;
                let nw = paint::measure(&a.name, t.font.caption, false)
                    .min(200)
                    .min(close_x - tx - t.spacing.sm);
                paint::text_elided(
                    &a.name,
                    tx,
                    y,
                    nw.max(0),
                    h,
                    t.font.caption,
                    false,
                    fg,
                    Align::Left,
                );
                if !a.detail.is_empty() {
                    let dx = tx + nw + t.spacing.sm;
                    paint::text_elided(
                        &a.detail,
                        dx,
                        y,
                        (close_x - dx - t.spacing.sm).max(0),
                        h,
                        t.font.caption,
                        false,
                        fg.darker(1.5),
                        Align::Left,
                    );
                }
                let close_hover = hov.is_some_and(|(_, c)| c);
                let cs = h - 6;
                if close_hover {
                    paint::fill(
                        close_x,
                        y + 3,
                        cs,
                        cs,
                        cs / 2,
                        t.palette.urgent.over(fill, 0.25),
                    );
                }
                paint::glyph(
                    glyph::CLOSE,
                    close_x,
                    y + 3,
                    cs,
                    cs,
                    t.font.caption,
                    if close_hover {
                        t.palette.urgent
                    } else {
                        fg.darker(1.3)
                    },
                );
            }
        });
    });
    let (m, cr, hv) = (c.d.attachments.clone(), cur.clone(), hover.clone());
    let me = c.clone();
    let at = move |b: &Widget, ex: i32, ey: i32| -> Option<(usize, bool)> {
        let items = m.to_vec();
        let h = chip_h();
        chip_rects(&items, b.w())
            .iter()
            .enumerate()
            .find_map(|(i, (rx, ry, rw, cx))| {
                let (x, y) = (b.x() + rx, b.y() + ry);
                (ex >= x && ex < x + rw && ey >= y && ey < y + h)
                    .then_some((i, ex >= b.x() + cx - 2))
            })
    };
    bar.handle(move |b, ev| {
        let n = me.d.attachments.len();
        match ev {
            Event::Focus => {
                cr.set(cr.get().min(n.saturating_sub(1)));
                b.redraw();
                true
            }
            Event::Unfocus => {
                b.redraw();
                true
            }
            Event::Enter | Event::Move => {
                let (ex, ey) = app::event_coords();
                let h = at(b, ex, ey);
                if hv.replace(h) != h {
                    b.redraw();
                }
                true
            }
            Event::Leave => {
                hv.set(None);
                b.redraw();
                true
            }
            Event::Push => {
                let (ex, ey) = app::event_coords();
                if let Some((i, close)) = at(b, ex, ey) {
                    cr.set(i);
                    if close {
                        me.remove_attachment(i);
                    } else {
                        me.open_attachment(i);
                    }
                }
                true
            }
            Event::KeyDown if n > 0 => {
                let k = app::event_key();
                if !(app::event_state()
                    & (Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta | Shortcut::Shift))
                    .is_empty()
                {
                    return false;
                }
                match k {
                    Key::Left | Key::Right => {
                        let c = cr.get() as isize + if k == Key::Right { 1 } else { -1 };
                        if c < 0 || c >= n as isize {
                            return false;
                        }
                        cr.set(c as usize);
                        b.redraw();
                        true
                    }
                    Key::Home | Key::End => {
                        cr.set(if k == Key::Home { 0 } else { n - 1 });
                        b.redraw();
                        true
                    }
                    Key::Delete | Key::BackSpace => {
                        me.remove_attachment(cr.get().min(n - 1));
                        let left = me.d.attachments.len();
                        if left == 0 {
                            let _ = me.area.clone().take_focus();
                        } else {
                            cr.set(cr.get().min(left - 1));
                        }
                        true
                    }
                    Key::Enter | Key::KPEnter => {
                        me.open_attachment(cr.get().min(n - 1));
                        true
                    }
                    k if k == Key::from_char(' ') => {
                        me.open_attachment(cr.get().min(n - 1));
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    });
    let (m, cr) = (c.d.attachments.clone(), cur.clone());
    let b2 = bar.clone();
    a11y::describe(&bar, move |n| {
        let items = m.to_vec();
        let rects = chip_rects(&items, b2.w());
        n.role(Role::List)
            .name(&crate::tr!("Attachments"))
            .focusable();
        let focused = crate::focus::is_focused(&b2);
        for (i, (a, (rx, ry, rw, _))) in items.iter().zip(rects).enumerate() {
            n.child(i as u64, |item| {
                item.role(Role::ListItem)
                    .name(&a.name)
                    .description(&a.detail)
                    .position(i + 1, items.len())
                    .bounds(b2.x() + rx, b2.y() + ry, rw, chip_h())
                    .action(Action::Click);
                if focused && cr.get() == i {
                    item.focused();
                }
                item.child(1, |x| {
                    x.role(Role::Button)
                        .name(&crate::tr!("Remove {name}", name = a.name))
                        .action(Action::Click);
                });
            });
        }
    });
    let me = c.clone();
    a11y::on_action(&bar, move |r| {
        let n = me.d.attachments.len();
        let Some(i) = r.path.first().map(|k| *k as usize).filter(|i| *i < n) else {
            return false;
        };
        match (r.action, r.path.len()) {
            (Action::Click, 1) => me.open_attachment(i),
            (Action::Click, 2) => me.remove_attachment(i),
            _ => return false,
        }
        true
    });
}

impl Composer {
    fn remove_attachment(&self, i: usize) {
        if let Some(a) = self.d.attachments.remove(i) {
            self.d.remove.call(a);
        }
    }

    fn open_attachment(&self, i: usize) {
        if let Some(a) = self.d.attachments.get(i) {
            self.d.open.call(a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Chip;

    const T: [(char, Trigger); 3] = [
        ('@', Trigger::WordStart),
        ('$', Trigger::WordStart),
        ('/', Trigger::LineStart),
    ];

    #[test]
    fn queries_follow_o3_rules() {
        let q = find_query("see @src/ma", 11, &[], &T).unwrap();
        assert_eq!(
            (q.trigger, q.text.as_str(), q.range.clone()),
            ('@', "src/ma", 4..11)
        );
        assert!(find_query("mail me@home", 12, &[], &T).is_none());
        assert_eq!(find_query("$", 1, &[], &T).unwrap().text, "");
        assert_eq!(find_query("hi\n/rev", 7, &[], &T).unwrap().range, 3..7);
        assert!(find_query("hi /rev", 7, &[], &T).is_none());
        assert!(find_query("@a b", 4, &[], &T).is_none());
        // Right after a chip: no query from the chip's own text.
        let chips = [Chip {
            range: 0..5,
            data: "x".into(),
        }];
        assert!(find_query("@main", 5, &chips, &T).is_none());
        assert_eq!(find_query("@main @x", 8, &chips, &T).unwrap().range, 6..8);
        // Multi-byte text before the trigger.
        assert_eq!(
            find_query("日本 @に", "日本 @に".len(), &[], &T)
                .unwrap()
                .text,
            "に"
        );
    }

    #[test]
    fn prompt_history_keeps_the_draft() {
        let list: Vec<String> = vec!["newest".into(), "older".into()];
        let mut h = PromptHistory::new();
        assert_eq!(h.step(&list, false, "draft"), None);
        assert_eq!(h.step(&list, true, "draft").as_deref(), Some("newest"));
        assert!(h.browsing());
        assert_eq!(h.step(&list, true, "newest").as_deref(), Some("older"));
        assert_eq!(h.step(&list, true, "older"), None);
        assert_eq!(h.step(&list, false, "older").as_deref(), Some("newest"));
        assert_eq!(h.step(&list, false, "newest").as_deref(), Some("draft"));
        assert!(!h.browsing());
        assert_eq!(h.step(&list, false, "draft"), None);
        // An edit resets: the edited text becomes the draft.
        h.step(&list, true, "draft");
        h.reset();
        assert_eq!(h.step(&list, true, "edited").as_deref(), Some("newest"));
        assert_eq!(h.step(&list, false, "newest").as_deref(), Some("edited"));
        assert_eq!(PromptHistory::new().step(&[], true, "x"), None);
    }

    #[test]
    fn file_lists_from_pastes_and_drops() {
        assert_eq!(
            parse_files("file:///home/a/My%20File.txt\r\nfile:///tmp/b\n", false),
            Some(vec![
                PathBuf::from("/home/a/My File.txt"),
                PathBuf::from("/tmp/b")
            ])
        );
        assert_eq!(
            parse_files("file://host/srv/x", false),
            Some(vec![PathBuf::from("/srv/x")])
        );
        assert_eq!(
            parse_files("file:///C:/Users/x.png", false),
            Some(vec![PathBuf::from("C:/Users/x.png")])
        );
        assert_eq!(
            parse_files("/tmp/a\n/tmp/b", true),
            Some(vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")])
        );
        assert_eq!(parse_files("/tmp/a", false), None);
        assert_eq!(parse_files("hello file:///x", false), None);
        assert_eq!(parse_files("", true), None);
        assert_eq!(size_text(512), "512 B");
        assert_eq!(size_text(12 * 1024), "12 KB");
        assert_eq!(size_text(3 * 1024 * 1024 + 400 * 1024), "3.4 MB");
    }
}
