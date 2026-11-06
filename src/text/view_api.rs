//! The transcript side of [`RichText`]: messages by key, model binding,
//! scroll anchors, follow-the-end, bottom inset, embeds, images, block
//! actions, find and the selection API.

use super::*;
use crate::model::ListModel;
use std::fmt::Display;
use std::hash::Hash;

impl Inner {
    /// Moves the current find match by `step` and scrolls it into view.
    pub(super) fn find_step(&mut self, step: i64, m: &mut dyn Metrics) -> Option<usize> {
        let f = self.find.as_mut()?;
        let n = f.matches.len();
        if n == 0 {
            f.current = None;
            return None;
        }
        let c = match f.current {
            Some(c) => (c as i64 + step).rem_euclid(n as i64) as usize,
            None if step >= 0 => 0,
            None => n - 1,
        };
        f.current = Some(c);
        let (k, r) = f.matches[c].clone();
        self.reveal_range(k, r, m);
        Some(c)
    }

    /// Scrolls so bytes `r` of block `k` are in view (centered when they
    /// were out of view).
    pub(super) fn reveal_range(&mut self, k: usize, r: Range<usize>, m: &mut dyn Metrics) {
        if k >= self.store.len() {
            return;
        }
        if self.width > 0 {
            self.ensure_layout(k, m);
        }
        let (y, h) = match self.laid(k) {
            Some((_, l)) => l
                .lines
                .iter()
                .find(|ln| (ln.end as usize) > r.start || ln.end as usize == l.lines.last().map_or(0, |x| x.end as usize))
                .map_or((0, l.height.min(self.usable_h())), |ln| (ln.y, ln.h)),
            None => (0, self.typo.line_h(self.typo.body)),
        };
        let at = self.heights.prefix(k) + self.gap(k) as i64 + y as i64 + self.pad as i64;
        let view = self.usable_h() as i64;
        if at < self.top || at + h as i64 > self.top + view {
            let max = self.max_top();
            self.set_top(at - (view - h as i64) / 2);
            self.stuck = self.stick && self.top >= max;
        }
    }

    /// Document plain-text offset of a position.
    fn offset_of(&self, p: TextPos) -> usize {
        let n = self.store.len();
        let mut at = 0;
        for k in 0..p.block.min(n) {
            at += self.store.text(k).len() + 1;
        }
        at + p.offset.min(if p.block < n { self.store.text(p.block).len() } else { 0 })
    }

    /// Position of a document plain-text offset.
    fn pos_of(&self, mut off: usize) -> TextPos {
        let n = self.store.len();
        for k in 0..n {
            let len = self.store.text(k).len();
            if off <= len {
                return TextPos { block: k, offset: off };
            }
            off -= len + 1;
        }
        match n.checked_sub(1) {
            Some(last) => TextPos {
                block: last,
                offset: self.store.text(last).len(),
            },
            None => TextPos::default(),
        }
    }

    fn set_image(&mut self, src: &str, data: ImageData) {
        let state = match data {
            ImageData::Size { width, height } => ImageState::Size(width, height),
            ImageData::Rgba { width, height, pixels } => {
                if width <= 0 || height <= 0 || pixels.len() < (width * height * 4) as usize {
                    ImageState::Failed
                } else {
                    match fltk::image::RgbImage::new(&pixels, width, height, fltk::enums::ColorDepth::Rgba8) {
                        Ok(img) => ImageState::Pixels(width, height, img),
                        Err(_) => ImageState::Failed,
                    }
                }
            }
            ImageData::Painted { height } => ImageState::Painted(height),
            ImageData::Failed => ImageState::Failed,
        };
        self.images.insert(Arc::from(src), state);
        for k in self.embed_blocks(src) {
            self.invalidate(k);
        }
        self.reanchor();
    }
}

impl RichText {
    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let r = f(&mut self.inner.borrow_mut());
        self.changed();
        r
    }

    // ------------------------------------------------------------ messages

    /// Sets message `key`: replaces its content when it exists (in place,
    /// the view stays still), else appends it. Markdown that extends the
    /// Markdown set before streams only the new text (O(tail)).
    pub fn set_message(&self, key: &str, content: Message) {
        self.with(|i| {
            let j = match i.key_index(key) {
                Some(j) => j,
                None => {
                    let j = i.msgs.len();
                    let start = i.store.len();
                    i.insert_msg(j, key, Vec::new(), None);
                    set_content(i, j, content);
                    i.reveal_from(start);
                    return;
                }
            };
            set_content(i, j, content);
        })
    }

    /// Inserts message `key` before message index `at` (`0`: the oldest
    /// place, `message_count()`: the end). An existing `key` moves there
    /// with the new content. Messages inserted above the view (older
    /// history) do not move what is on screen.
    pub fn insert_message(&self, at: usize, key: &str, content: Message) {
        self.with(|i| insert(i, at, key, content))
    }

    /// Prepends older messages (oldest first) in one go; the view stays
    /// still.
    pub fn prepend_messages(&self, older: impl IntoIterator<Item = (String, Message)>) {
        self.with(|i| {
            for (n, (key, content)) in older.into_iter().enumerate() {
                insert(i, n, &key, content);
            }
        })
    }

    /// Removes message `key`; false when there is none.
    pub fn remove_message(&self, key: &str) -> bool {
        self.with(|i| match i.key_index(key) {
            Some(j) => {
                i.remove_msg(j);
                true
            }
            None => false,
        })
    }

    /// Moves message `key` to message index `to`.
    pub fn move_message(&self, key: &str, to: usize) -> bool {
        self.with(|i| {
            let Some(j) = i.key_index(key) else { return false };
            if j == to {
                return true;
            }
            let blocks: Vec<Block> = {
                let s = i.msg_start(j);
                (s..s + i.msg_len(j)).map(|k| i.store.block(k)).collect()
            };
            let stream = i.msgs[j].stream.take();
            let src = i.msgs[j].src;
            i.remove_msg(j);
            let to = to.min(i.msgs.len());
            i.insert_msg(to, key, blocks, stream);
            i.msgs[to].src = src;
            true
        })
    }

    /// Streams Markdown into message `key` (appended if missing).
    pub fn push_message_markdown(&self, key: &str, more: &str) {
        let tail = {
            let mut i = self.inner.borrow_mut();
            let (j, new) = match i.key_index(key) {
                Some(j) => (j, false),
                None => {
                    let j = i.msgs.len();
                    i.insert_msg(j, key, Vec::new(), None);
                    (j, true)
                }
            };
            i.push_msg(j, more);
            i.msgs[j].src = None;
            (!new && i.tail_push(j)).then_some(i.stream_pace)
        };
        // Growing the last message while following the end moves pixels.
        damage_push(&mut self.widget.clone(), tail);
    }

    /// Closes the stream of message `key` (frees its parser state; a later
    /// `set_message` re-parses it whole).
    pub fn finish_message(&self, key: &str) {
        {
            let mut i = self.inner.borrow_mut();
            if let Some(j) = i.key_index(key) {
                i.end_stream(j);
            }
        }
        self.widget.clone().redraw();
    }

    pub fn message_count(&self) -> usize {
        self.inner.borrow().msgs.len()
    }

    /// Message keys in order.
    pub fn message_keys(&self) -> Vec<String> {
        self.inner.borrow().msgs.iter().map(|m| m.key.to_string()).collect()
    }

    /// Index of message `key`.
    pub fn message_index(&self, key: &str) -> Option<usize> {
        self.inner.borrow_mut().key_index(key)
    }

    /// Block range of message `key`.
    pub fn message_blocks(&self, key: &str) -> Option<Range<usize>> {
        let mut i = self.inner.borrow_mut();
        let j = i.key_index(key)?;
        let s = i.msg_start(j);
        Some(s..s + i.msg_len(j))
    }

    /// Window rectangle `(x, y, w, h)` of message `key` (all its blocks,
    /// not clipped) while some of it is in view; None when it is out of
    /// view or unknown. Scroll glides included (tests, the motion gate).
    pub fn message_rect(&self, key: &str) -> Option<(i32, i32, i32, i32)> {
        let r = self.message_blocks(key).filter(|r| !r.is_empty())?;
        let i = self.inner.borrow();
        let wy = self.widget.y();
        let top = i.screen_y(r.start, wy);
        let bottom = (wy as i64 + i.pad as i64 + i.heights.prefix(r.end) - i.top) as i32;
        let view_h = if i.view_h > 0 { i.view_h } else { self.widget.h() };
        if bottom <= wy || top >= wy + view_h {
            return None;
        }
        Some((self.widget.x(), top, self.widget.w(), bottom - top))
    }

    /// Key of the message that holds block `block`.
    pub fn message_at(&self, block: usize) -> Option<String> {
        let i = self.inner.borrow();
        i.msg_of(block).map(|j| i.msgs[j].key.to_string())
    }

    /// Byte range of message `key` in [`plain_text`](Self::plain_text).
    pub fn message_text_range(&self, key: &str) -> Option<Range<usize>> {
        let r = self.message_blocks(key)?;
        let i = self.inner.borrow();
        let a = i.offset_of(TextPos { block: r.start, offset: 0 });
        if r.is_empty() {
            return Some(a..a);
        }
        let last = r.end - 1;
        let b = i.offset_of(TextPos {
            block: last,
            offset: i.store.text(last).len(),
        });
        Some(a..b)
    }

    /// Makes this view follow `model`: one message per item, keyed by the
    /// item's key (its `Display`), content from `render`. Updates re-render
    /// only the items that changed (`Changes::updated`); inserts, removals
    /// and moves become message inserts, removals and moves. Older history
    /// inserted at the top keeps the view still. Streaming rows: render
    /// `Message::Markdown` with the full text; an extended text streams.
    ///
    /// ```no_run
    /// use fltk_omarchy::{model::ListModel, text::{Message, RichText}};
    /// #[derive(Clone, PartialEq)]
    /// struct Row { id: String, md: String }
    /// let rows = ListModel::new(|r: &Row| r.id.clone());
    /// let view = RichText::new().stick_to_bottom(true);
    /// view.bind_model(&rows, |r| Message::markdown(&r.md));
    /// rows.push(Row { id: "m1".into(), md: "Hello".into() });
    /// rows.patch(&"m1".to_string(), |r| r.md.push_str(", world")); // streams ", world"
    /// ```
    pub fn bind_model<K, T>(&self, model: &ListModel<K, T>, render: impl Fn(&T) -> Message + 'static)
    where
        K: Clone + Eq + Hash + Display + 'static,
        T: Clone + PartialEq + 'static,
    {
        let render = Rc::new(render);
        sync_all(self, model, &*render);
        let weak = Rc::downgrade(&self.inner);
        let w = self.widget.clone();
        let hooks = self.hooks.clone();
        model.listen(move |m, changes| {
            let Some(inner) = weak.upgrade().filter(|_| !w.was_deleted()) else { return false };
            let rt = RichText {
                widget: w.clone(),
                inner,
                hooks: hooks.clone(),
            };
            if changes.is_structural() {
                sync_all(&rt, m, &*render);
            } else {
                for id in changes.updated().iter().chain(changes.inserted()) {
                    if let Some((Some(key), content)) = m.with_id(*id, |t| (m.key_of(*id), render(t))) {
                        rt.set_message(&key.to_string(), content);
                    }
                }
            }
            true
        });
    }

    // -------------------------------------------------- scroll and follow

    /// Where the view is, by message key (see [`ScrollAnchor`]); None when
    /// empty.
    pub fn scroll_anchor(&self) -> Option<ScrollAnchor> {
        let i = self.inner.borrow();
        if i.store.len() == 0 {
            return None;
        }
        let at_end = i.stuck || i.at_bottom();
        let (k, _) = i.locate(i.top);
        let j = i.msg_of(k)?;
        let y = i.top - i.pad as i64;
        let offset = y - i.heights.prefix(i.msg_start(j));
        Some(ScrollAnchor {
            key: i.msgs[j].key.to_string(),
            offset,
            at_end,
        })
    }

    /// Restores a saved anchor: the end (following it) for `at_end`, else
    /// message `key` at `offset` pixels above the view's top, kept there as
    /// blocks get measured. False when `key` is gone (the view goes to the
    /// end).
    pub fn restore_scroll(&self, a: &ScrollAnchor) -> bool {
        self.with(|i| {
            let j = if a.at_end { None } else { i.key_index(&a.key) };
            match j {
                Some(j) => {
                    i.stuck = false;
                    i.anchor = (i.msg_start(j).min(i.store.len().saturating_sub(1)), a.offset);
                    i.motion.jump(i.top);
                    i.reanchor();
                    i.motion.jump(i.top);
                    true
                }
                None => {
                    i.stuck = true;
                    let max = i.max_top();
                    i.set_top(max);
                    a.at_end
                }
            }
        })
    }

    /// Follows the end again (a "jump to latest" button, a sent message):
    /// stays at the end as content grows until the user scrolls up.
    pub fn follow_end(&self) {
        self.with(|i| {
            i.stuck = true;
            let max = i.max_top();
            let view = i.view_h.max(1) as i64;
            if !crate::anim::enabled() || i.top >= max {
                i.set_top(max);
                return;
            }
            // A long way: cut to two screens above the end, then glide (as
            // Rune's scroll_to_bottom does), so the trip costs a few frames.
            if max - i.top > 2 * view {
                i.set_top(max - 2 * view);
            }
            let top = i.top;
            i.motion.wheel(top, (max - top) as f64, max);
        })
    }

    /// Called with true when the view is more than `min_px` above the end
    /// and with false when it comes back within it: show a "jump to latest"
    /// button only when it is worth it (it does not flicker while the end
    /// grows or the reader is a line away).
    pub fn on_far_from_bottom(self, min_px: i32, f: impl FnMut(bool) + 'static) -> Self {
        *self.hooks.far.borrow_mut() = Some(Box::new(f));
        self.inner.borrow_mut().far = Some((min_px.max(0) as i64, false));
        self
    }

    /// True while the view follows the end.
    pub fn is_following(&self) -> bool {
        self.inner.borrow().stuck
    }

    /// Called when following the end starts or stops (show a "jump to
    /// latest" button while it is off).
    pub fn on_follow_change(self, f: impl FnMut(bool) + 'static) -> Self {
        *self.hooks.follow.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Space under the last message for a composer drawn over the view.
    pub fn bottom_inset(self, px: i32) -> Self {
        self.set_bottom_inset(px);
        self
    }

    /// Changes the bottom inset (the composer grew): the end stays at the
    /// end, otherwise the view stays still.
    pub fn set_bottom_inset(&self, px: i32) {
        let px = px.max(0);
        let keep = self.with(|i| {
            let old = i.inset;
            // Shrinking while following: keep the room (the rows do not dip
            // after a send); growing uses up kept room first.
            if px < old && i.stuck {
                i.slack += (old - px) as i64;
            } else if px > old {
                i.slack = (i.slack - (px - old) as i64).max(0);
            }
            i.inset = px;
            i.reanchor();
            i.slack > 0
        });
        if keep {
            self.release_slack_later();
        }
    }

    /// Releases the kept room 120 ms after the content stopped growing,
    /// gliding to the new end.
    fn release_slack_later(&self) {
        const WAIT: f64 = 0.12;
        let (inn, w) = (Rc::downgrade(&self.inner), self.widget.clone());
        app::add_timeout3(WAIT, move |h| {
            let Some(inn) = inn.upgrade() else { return };
            if w.was_deleted() {
                return;
            }
            let mut i = inn.borrow_mut();
            if i.slack == 0 {
                return;
            }
            if let Some(t) = i.grown.1 {
                let left = WAIT - t.elapsed().as_secs_f64();
                if left > 0.0 {
                    app::repeat_timeout3(left, h);
                    return;
                }
            }
            i.slack = 0;
            let max = i.max_top();
            if i.stuck && i.top > max {
                let top = i.top;
                i.motion.wheel(top, (max - top) as f64, max);
            } else {
                i.reanchor();
            }
            drop(i);
            w.clone().redraw();
        });
    }

    /// Scrolls so position `p` is in view.
    pub fn scroll_to_pos(&self, p: TextPos) {
        self.with(|i| i.reveal_range(p.block, p.offset..p.offset, &mut FltkMetrics::default()))
    }

    /// Scrolls message `key` to the top of the view.
    pub fn scroll_to_message(&self, key: &str) -> bool {
        self.with(|i| {
            let Some(j) = i.key_index(key) else { return false };
            let y = i.heights.prefix(i.msg_start(j)) + i.pad as i64 - i.pad as i64;
            i.stuck = false;
            i.set_top(y);
            true
        })
    }

    // -------------------------------------------------------------- embeds

    /// Draws embed blocks (and images resolved as `ImageData::Painted`).
    /// Runs during the paint: draw only, do not change the view from it.
    pub fn embed_painter(self, f: impl FnMut(&mut EmbedPaint) + 'static) -> Self {
        self.inner.borrow_mut().painter = Some(Rc::new(RefCell::new(f)));
        self
    }

    /// Called with an embed's id when it enters (`true`) or leaves the
    /// view: load media lazily, pause what is not seen.
    pub fn on_embed_visibility(self, f: impl FnMut(&str, bool) + 'static) -> Self {
        self.inner.borrow_mut().track = true;
        *self.hooks.visibility.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Makes embeds Tab stops; Enter, Space or a click calls `f` with the
    /// id and, for a click, the position inside the embed.
    pub fn on_embed_activate(self, f: impl FnMut(&str, Option<(i32, i32)>) + 'static) -> Self {
        self.inner.borrow_mut().embed_focus = true;
        *self.hooks.embed.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Changes the height of embed `id`; what is on screen stays still
    /// (or the end stays at the end). False when there is no such embed.
    pub fn set_embed_height(&self, id: &str, height: i32) -> bool {
        self.with(|i| {
            let ks = i.embed_blocks(id);
            let mut any = false;
            for k in ks {
                if i.store.is_embed(k) && i.store.set_embed_height(k, height) {
                    i.invalidate(k);
                    any = true;
                }
            }
            i.reanchor();
            any
        })
    }

    /// Window rectangle of embed `id` when it is in view.
    pub fn embed_rect(&self, id: &str) -> Option<(i32, i32, i32, i32)> {
        let mut i = self.inner.borrow_mut();
        let vis = i.visible();
        let k = i.embed_blocks(id).into_iter().find(|k| vis.contains(k))?;
        let (_, l) = i.laid(k)?;
        let (left, w, h) = (l.left, l.box_w, l.height);
        Some((self.widget.x() + i.pad + left, i.screen_y(k, self.widget.y()), w, h))
    }

    /// Hosts `widget` over embed `id`: the view moves it over the embed
    /// while the embed is in view (clipped to the view, above the bottom
    /// inset) and hides it otherwise. The widget is moved into an overlay
    /// group of the view's window. Keyboard: it is a normal fltk widget,
    /// reached with Tab after the view.
    pub fn host_widget(&self, id: &str, widget: &impl WidgetExt) {
        let mut wd = widget.as_base_widget();
        wd.hide();
        self.with(|i| {
            i.hosted.retain(|(k, w)| **k != *id && !w.was_deleted());
            i.hosted.push((Arc::from(id), wd));
        })
    }

    // -------------------------------------------------------------- images

    /// Called with an image source the first time its block comes into
    /// view; answer with [`set_image`](Self::set_image) (now or later).
    /// Turns [`MarkdownOptions::images`] on for Markdown added from now on.
    pub fn on_image(self, f: impl FnMut(&str) + 'static) -> Self {
        {
            let mut i = self.inner.borrow_mut();
            i.want_images = true;
            i.opts.images = true;
        }
        *self.hooks.image.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Resolves image `src`: a size (placeholder), pixels, a painted embed
    /// or a failure (the alt text stays). Every block of that source
    /// updates; the view stays still.
    pub fn set_image(&self, src: &str, data: ImageData) {
        self.with(|i| i.set_image(src, data))
    }

    /// Images are at most this tall (default 320 px), and at most the
    /// column wide.
    pub fn image_max_height(self, px: i32) -> Self {
        {
            let mut i = self.inner.borrow_mut();
            i.image_max_h = px.max(16);
            i.relayout_all();
        }
        self
    }

    // ------------------------------------------------------------- mermaid

    /// Draws closed ```` ```mermaid ```` code blocks as native diagrams
    /// (feature `mermaid`): flowcharts, sequence and state diagrams under
    /// the code header, which keeps the language, the copy button and a
    /// "Show source" action ([`crate::mermaid::SHOW_SOURCE`]; then "Show
    /// diagram", [`crate::mermaid::SHOW_DIAGRAM`]; both also reach
    /// [`on_block_action`](Self::on_block_action)). A fence still
    /// streaming shows its code and becomes a diagram when it closes; a
    /// source that does not render stays code. Diagrams are laid out for
    /// the column width and cached by source until the width or the theme
    /// change.
    #[cfg(feature = "mermaid")]
    pub fn native_mermaid(self, on: bool) -> Self {
        {
            let mut i = self.inner.borrow_mut();
            i.mermaid = on.then(Default::default);
            i.relayout_all();
        }
        self
    }

    /// The diagram drawn for Mermaid source `src` (tests, tours).
    #[cfg(feature = "mermaid")]
    pub fn mermaid_diagram(&self, src: &str) -> Option<crate::mermaid::Diagram> {
        self.inner.borrow().mermaid.as_ref().and_then(|n| n.diagram(src).cloned())
    }

    // ------------------------------------------------------------- actions

    /// App actions in each code block's header, before the copy button:
    /// `f(lang, code)` returns them (o3: "Run in terminal" when the block
    /// has a command, "Show diagram" for Mermaid). Results reach
    /// [`on_block_action`](Self::on_block_action).
    pub fn code_actions(self, f: impl Fn(&str, &str) -> Vec<BlockAction> + 'static) -> Self {
        {
            let mut i = self.inner.borrow_mut();
            i.code_actions = Some(Rc::new(f));
            i.relayout_all();
        }
        self
    }

    /// A footer under each table with "Copy as Markdown" and "Copy as CSV"
    /// (built in: they copy, then report to `on_block_action`).
    pub fn table_actions(self, on: bool) -> Self {
        {
            let mut i = self.inner.borrow_mut();
            i.table_actions = on;
            i.relayout_all();
        }
        self
    }

    /// More table footer actions, after the built-in copies (turns the
    /// footer on).
    pub fn extra_table_actions(self, f: impl Fn(&Block) -> Vec<BlockAction> + 'static) -> Self {
        {
            let mut i = self.inner.borrow_mut();
            i.table_actions = true;
            i.extra_table_actions = Some(Rc::new(f));
            i.relayout_all();
        }
        self
    }

    /// Called when a code or table action is used (click, Enter, Space),
    /// including the built-in copies.
    pub fn on_block_action(self, f: impl FnMut(&BlockActionEvent) + 'static) -> Self {
        *self.hooks.action.borrow_mut() = Some(Box::new(f));
        self
    }

    /// What Tab focused inside the view other than a link: an action id
    /// (`BlockAction::COPY` for a code copy button) or `embed:<id>`.
    pub fn focused_action(&self) -> Option<String> {
        let i = self.inner.borrow();
        match i.focus? {
            Target::Copy { .. } => Some(BlockAction::COPY.to_string()),
            Target::Action { block, slot } => i.actions_of(block).get(slot).map(|a| a.id.clone()),
            Target::Embed { block } => i.store.embed_id(block).map(|s| format!("embed:{s}")),
            Target::Link { .. } => None,
        }
    }

    /// Window rectangle of action `id` of block `block` when it is laid
    /// out (tests, tours).
    pub fn action_rect(&self, block: usize, id: &str) -> Option<(i32, i32, i32, i32)> {
        let i = self.inner.borrow();
        let (b, l) = i.laid(block)?;
        let bx = self.widget.x() + i.pad;
        let by = i.screen_y(block, self.widget.y());
        let hh = i.typo.code_header;
        if id == BlockAction::COPY && b.is_code() {
            return Some(copy_rect(l, &i.typo, bx, by));
        }
        let a = i.actions_of(block).iter().find(|a| a.id == id)?;
        let y = if b.is_code() { by } else { by + l.height - hh };
        Some((bx + a.x, y + 1, a.w, hh - 2))
    }

    // ---------------------------------------------------------- selection

    /// Window rectangle around the part of the selection in view (place a
    /// floating "Quote" button over it); None when nothing selected is in
    /// view.
    pub fn selection_rect(&self) -> Option<(i32, i32, i32, i32)> {
        let i = self.inner.borrow();
        let (a, b) = i.sel_range()?;
        let mut m = FltkMetrics::default();
        let (wx, wy) = (self.widget.x(), self.widget.y());
        let (top, bottom) = (wy, wy + i.usable_h());
        let mut r: Option<(i32, i32, i32, i32)> = None;
        let mut add = |x: i32, y: i32, w: i32, h: i32| {
            let (y0, y1) = (y.max(top), (y + h).min(bottom));
            if y1 <= y0 || w <= 0 {
                return;
            }
            r = Some(match r {
                None => (x, y0, x + w, y1),
                Some((ax, ay, bx, by)) => (ax.min(x), ay.min(y0), bx.max(x + w), by.max(y1)),
            });
        };
        for k in i.visible() {
            if k < a.block || k > b.block {
                continue;
            }
            let Some((bl, l)) = i.laid(k) else { continue };
            let by = i.screen_y(k, wy);
            let bx = wx + i.pad;
            let text = bl.text();
            let s = if k == a.block { a.offset } else { 0 };
            let e = if k == b.block { b.offset } else { text.len() + 1 };
            if l.lines.is_empty() {
                add(bx + l.left, by, l.box_w, l.height);
                continue;
            }
            for (x, y, w, h) in l.rects(text, s.min(text.len())..e.min(text.len() + 1), &mut m) {
                add(bx + x, by + y, w, h);
            }
        }
        r.map(|(x0, y0, x1, y1)| (x0, y0, x1 - x0, y1 - y0))
    }

    /// Called when the selection changes (read it with
    /// [`selected_text`](Self::selected_text) / `selection_rect`).
    pub fn on_selection_change(self, f: impl FnMut() + 'static) -> Self {
        *self.hooks.selection.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Selects bytes `range` of [`plain_text`](Self::plain_text).
    pub fn select_text(&self, range: Range<usize>) {
        self.with(|i| {
            let (a, b) = (i.pos_of(range.start), i.pos_of(range.end));
            i.sel = Some((a, b));
        })
    }

    /// Plain-text offset of a position (see [`plain_text`](Self::plain_text)).
    pub fn text_offset(&self, p: TextPos) -> usize {
        self.inner.borrow().offset_of(p)
    }

    /// Position of a plain-text offset.
    pub fn text_pos(&self, offset: usize) -> TextPos {
        self.inner.borrow().pos_of(offset)
    }

    // --------------------------------------------------------------- find

    /// Highlights every match of `query` (case-insensitive) and makes the
    /// first one at or below the view's top current (scrolled into view).
    /// Matches follow later content (streams, new messages). Returns the
    /// count; an empty query clears.
    pub fn find(&self, query: &str) -> usize {
        self.with(|i| {
            if query.is_empty() {
                i.find = None;
                return 0;
            }
            let mut matches = Vec::new();
            for k in 0..i.store.len() {
                for r in find_in(i.store.text(k), query) {
                    matches.push((k, r));
                }
            }
            let (k0, _) = i.locate(i.top);
            let first = matches.iter().position(|(k, _)| *k >= k0).or((!matches.is_empty()).then_some(0));
            let n = matches.len();
            i.find = Some(Find {
                query: query.to_string(),
                matches,
                current: None,
            });
            if let Some(c) = first {
                let (k, r) = i.find.as_ref().map(|f| f.matches[c].clone()).unwrap_or_default();
                if let Some(f) = i.find.as_mut() {
                    f.current = Some(c);
                }
                i.reveal_range(k, r, &mut FltkMetrics::default());
            }
            n
        })
    }

    /// Next match (wraps); its index.
    pub fn find_next(&self) -> Option<usize> {
        self.with(|i| i.find_step(1, &mut FltkMetrics::default()))
    }

    /// Previous match (wraps); its index.
    pub fn find_prev(&self) -> Option<usize> {
        self.with(|i| i.find_step(-1, &mut FltkMetrics::default()))
    }

    pub fn find_count(&self) -> usize {
        self.inner.borrow().find.as_ref().map_or(0, |f| f.matches.len())
    }

    /// Index of the current match.
    pub fn find_current(&self) -> Option<usize> {
        self.inner.borrow().find.as_ref().and_then(|f| f.current)
    }

    /// The current match as (start, end) positions.
    pub fn find_match(&self) -> Option<(TextPos, TextPos)> {
        let i = self.inner.borrow();
        let f = i.find.as_ref()?;
        let (k, r) = f.matches.get(f.current?)?;
        Some((TextPos { block: *k, offset: r.start }, TextPos { block: *k, offset: r.end }))
    }

    pub fn clear_find(&self) {
        self.with(|i| i.find = None)
    }
}

fn set_content(i: &mut Inner, j: usize, content: Message) {
    match content {
        Message::Markdown(md) => i.set_msg_markdown(j, &md),
        Message::Blocks(b) => {
            i.msgs[j].stream = None;
            i.msgs[j].src = None;
            i.set_msg_blocks(j, 0, b);
        }
    }
}

fn insert(i: &mut Inner, at: usize, key: &str, content: Message) {
    if let Some(j) = i.key_index(key) {
        i.remove_msg(j);
    }
    let at = at.min(i.msgs.len());
    i.insert_msg(at, key, Vec::new(), None);
    set_content(i, at, content);
}

/// Puts the view's messages in the model's order, rendering only new ones
/// (and, after a reset, every item: the content check skips equal ones).
fn sync_all<K, T>(rt: &RichText, m: &ListModel<K, T>, render: &dyn Fn(&T) -> Message)
where
    K: Clone + Eq + Hash + Display + 'static,
    T: Clone + PartialEq + 'static,
{
    let mut i = rt.inner.borrow_mut();
    let mut want: Vec<String> = Vec::with_capacity(m.len());
    for n in 0..m.len() {
        if let Some(k) = m.key_at(n) {
            want.push(k.to_string());
        }
    }
    let wanted: std::collections::HashSet<&str> = want.iter().map(String::as_str).collect();
    // Removals first (from the end: fewer bytes move).
    for j in (0..i.msgs.len()).rev() {
        if !wanted.contains(i.msgs[j].key.to_string().as_str()) {
            i.remove_msg(j);
        }
    }
    for (n, key) in want.iter().enumerate() {
        let here = i.msgs.get(n).is_some_and(|m| m.key.is(key));
        if here {
            continue;
        }
        match i.key_index(key) {
            Some(j) => {
                // Moved: take its blocks to the new place.
                let s = i.msg_start(j);
                let blocks: Vec<Block> = (s..s + i.msg_len(j)).map(|k| i.store.block(k)).collect();
                let stream = i.msgs[j].stream.take();
                let src = i.msgs[j].src;
                i.remove_msg(j);
                i.insert_msg(n, key.as_str(), blocks, stream);
                i.msgs[n].src = src;
            }
            None => {
                let content = m.with_at(n, render);
                i.insert_msg(n, key.as_str(), Vec::new(), None);
                if let Some(c) = content {
                    set_content(&mut i, n, c);
                }
            }
        }
    }
    drop(i);
    rt.changed();
}

/// Without the `a11y` feature: messages near the view as rows for
/// `testkit::tree` (kept only while the kit is armed).
#[cfg(not(feature = "a11y"))]
pub(super) fn testkit_rows(rt: &RichText) {
    let v = rt.clone();
    crate::a11y::describe(&rt.widget.clone(), move |n| {
        n.role(crate::a11y::Role::Document);
        for (j, key) in v.message_keys().iter().enumerate() {
            if let Some((x, y, w, h)) = v.message_rect(key) {
                n.child(j as u64, |c| {
                    c.role(crate::a11y::Role::Group).name(key).bounds(x, y, w, h);
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::layout::FakeMetrics;

    fn view() -> Inner {
        let mut i = Inner::new();
        i.width = 400;
        i.view_h = 200;
        i.measured_avg = true;
        i
    }

    fn lay(i: &mut Inner) {
        i.ensure_visible(&mut FakeMetrics);
    }

    fn md(i: &mut Inner, key: &str, text: &str) {
        let j = match i.key_index(key) {
            Some(j) => j,
            None => {
                let j = i.msgs.len();
                i.insert_msg(j, key, Vec::new(), None);
                j
            }
        };
        set_content(i, j, Message::markdown(text));
    }

    /// Heights equal the sum of each block's gap and body, counts the
    /// messages' blocks, keys their index.
    fn consistent(i: &mut Inner) {
        let n = i.store.len();
        assert_eq!(i.heights.len(), n);
        for k in 0..n {
            let want = (i.gap(k) + i.body_h(k)) as i64;
            assert_eq!(i.heights.get(k), want, "height of block {k}");
        }
        assert_eq!(i.counts.prefix(i.counts.len()) as usize, n, "counts");
        for j in 0..i.msgs.len() {
            let key = i.msgs[j].key.to_string();
            assert_eq!(i.key_index(&key), Some(j));
        }
    }

    fn texts(i: &Inner, key: &str) -> Vec<String> {
        let j = i.msgs.iter().position(|m| m.key.is(key)).unwrap();
        let s = i.msg_start(j);
        (s..s + i.msg_len(j)).map(|k| i.store.text(k).to_string()).collect()
    }

    #[test]
    fn keyed_messages_insert_replace_move_remove() {
        let mut i = view();
        md(&mut i, "a", "# A\n\nfirst");
        md(&mut i, "b", "second **b**");
        insert(&mut i, 0, "z", Message::markdown("oldest"));
        insert(&mut i, 2, "card", Message::embed("card-1", 90, "Ran 3 commands"));
        lay(&mut i);
        let keys: Vec<String> = i.msgs.iter().map(|m| m.key.to_string()).collect();
        assert_eq!(keys, ["z", "a", "card", "b"]);
        consistent(&mut i);
        // Replace in the middle: only that message's blocks change.
        md(&mut i, "a", "only one paragraph");
        assert_eq!(texts(&i, "a"), ["only one paragraph"]);
        assert_eq!(texts(&i, "b"), ["second b"]);
        consistent(&mut i);
        // Move and remove.
        let j = i.key_index("card").unwrap();
        i.remove_msg(j);
        insert(&mut i, 0, "card", Message::embed("card-1", 90, "Ran 3 commands"));
        assert!(i.msgs[0].key.is("card"));
        assert_eq!(i.embed_blocks("card-1"), vec![0]);
        let j = i.key_index("z").unwrap();
        i.remove_msg(j);
        assert!(i.key_index("z").is_none());
        assert_eq!(i.embed_blocks("card-1"), vec![0]);
        lay(&mut i);
        consistent(&mut i);
    }

    #[test]
    fn set_message_streams_when_the_text_extends() {
        let mut i = view();
        md(&mut i, "a", "intro");
        let full = "Hello **world**.\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\nMore text";
        let mut at = 0;
        while at < full.len() {
            at = (at + 7).min(full.len());
            while !full.is_char_boundary(at) {
                at += 1;
            }
            md(&mut i, "b", &full[..at]);
        }
        assert!(i.msgs[1].stream.as_ref().is_some_and(|s| s.committed() > 0), "streamed, not re-parsed");
        let want: Vec<String> = crate::text::markdown::parse(full).iter().map(|b| b.text().to_string()).collect();
        assert_eq!(texts(&i, "b"), want);
        assert_eq!(texts(&i, "a"), ["intro"]);
        // A value that does not extend re-parses.
        md(&mut i, "b", "replaced");
        assert_eq!(texts(&i, "b"), ["replaced"]);
        lay(&mut i);
        consistent(&mut i);
    }

    #[test]
    fn prepend_and_embed_growth_keep_the_view_still() {
        let mut i = view();
        for n in 0..40 {
            md(&mut i, &format!("m{n}"), &format!("Message {n} with a few words in it.\n\nSecond paragraph of {n}."));
        }
        lay(&mut i);
        i.set_top(600);
        lay(&mut i);
        let (k, off) = i.locate(i.top);
        let text = i.store.text(k).to_string();
        let screen = |i: &Inner, k: usize| i.heights.prefix(k) - i.top;
        let before = screen(&i, k);
        // Older history above.
        for n in 0..5 {
            insert(&mut i, n, &format!("old{n}"), Message::markdown("An older message\n\nwith two paragraphs"));
        }
        i.reanchor();
        lay(&mut i);
        let k2 = (0..i.store.len()).find(|&x| i.store.text(x) == text).unwrap();
        assert_eq!(k2, k + 10);
        assert_eq!(screen(&i, k2), before, "prepend moved the view");
        assert_eq!(i.locate(i.top), (k2, off));
        // A card above the view grows.
        insert(&mut i, 1, "card", Message::embed("c", 40, "card"));
        i.reanchor();
        lay(&mut i);
        let k3 = k2 + 1;
        let at = screen(&i, k3);
        let b = i.embed_blocks("c")[0];
        i.store.set_embed_height(b, 400);
        i.invalidate(b);
        i.reanchor();
        lay(&mut i);
        assert_eq!(screen(&i, k3), at, "embed growth above moved the view");
        consistent(&mut i);
        // Following the end: growth keeps the end in view.
        i.stuck = true;
        lay(&mut i);
        md(&mut i, "tail", "a new message at the end\n\nwith more");
        lay(&mut i);
        assert!(i.at_bottom());
    }

    #[test]
    fn find_follows_new_content() {
        let mut i = view();
        md(&mut i, "a", "Alpha beta ALPHA\n\nnothing");
        md(&mut i, "b", "alpha again");
        let mut matches = Vec::new();
        for k in 0..i.store.len() {
            for r in find_in(i.store.text(k), "alpha") {
                matches.push((k, r));
            }
        }
        i.find = Some(Find {
            query: "alpha".into(),
            matches,
            current: None,
        });
        assert_eq!(i.find.as_ref().unwrap().matches.len(), 3);
        md(&mut i, "c", "the last alpha");
        assert_eq!(i.find.as_ref().unwrap().matches.len(), 4);
        let j = i.key_index("a").unwrap();
        i.remove_msg(j);
        let f = i.find.as_ref().unwrap();
        assert_eq!(f.matches.len(), 2);
        assert!(f.matches.iter().all(|(k, r)| i.store.text(*k)[r.clone()].eq_ignore_ascii_case("alpha")));
        assert_eq!(i.find_step(1, &mut FakeMetrics), Some(0));
        assert_eq!(i.find_step(1, &mut FakeMetrics), Some(1));
        assert_eq!(i.find_step(1, &mut FakeMetrics), Some(0), "wraps");
        assert_eq!(i.find_step(-1, &mut FakeMetrics), Some(1));
        assert_eq!(find_in("Ünïcode ÜNÏ", "ünï"), vec![0..5, 10..15]);
        assert!(find_in("abc", "").is_empty());
    }

    #[test]
    fn positions_and_offsets_round_trip() {
        let mut i = view();
        md(&mut i, "a", "one\n\ntwo");
        md(&mut i, "b", "three");
        for off in 0..=13 {
            assert_eq!(i.offset_of(i.pos_of(off)), off.min(13));
        }
        assert_eq!(i.pos_of(4), TextPos { block: 1, offset: 0 });
    }
}

