# Keyed tab strips

Use `TabBar::from_model` for dynamic tabs. It returns `KeyedTabBar<K>` and
follows a `ListModel<K, T>`; keys can be strings, UUIDs or compound values
such as `(pane_id, tab_id)`. Titles are presentation, not identity. The
index-based `TabBar::new` remains available for fixed strips.

```rust
use fltk_omarchy::{model::ListModel, widgets::*, State, tr};

type TabKey = (u64, u64);
#[derive(Clone, PartialEq)]
struct TabRow { key: TabKey, title: String }
let rows = ListModel::new(|row: &TabRow| row.key);
rows.push(TabRow { key: (1, 7), title: tr!("Work") });
let selected = State::new(Some((1, 7)));
let moves = rows.clone();
let tabs = TabBar::from_model(&rows, &selected, |row| row.title.clone())
    .on_close(|key| println!("Close request: {key:?}"))
    .on_context(|request| println!("Context request: {:?}", request.key))
    .reorderable(move |request| { request.apply_to(&moves); });
// body.fit(&*tabs);
```

Replace the logging callbacks with the app's existing close/menu
controller. Close and context requests do not select or remove a tab.
Reorder requests do not mutate the model automatically: forward them to a
controller or apply them explicitly as above. Pane content remains in the
app's registry keyed by `TabKey`; changing the strip does not rebuild it.

## Selection and updates

The `State<Option<K>>` belongs to the app. Model patches, moves, snapshots,
close requests and context requests never rewrite it. If its key is
absent, no tab is highlighted; the strip does not choose a replacement.
The close controller decides the next active pane/tab, preferably together
with its model edits in a transaction/batch. `select_key(&key)` is an
explicit selection command and ignores an unknown key.

Use `patch`/`upsert`/`move_key` or `sync_from` on the model. Existing keys
keep their model `ItemId`, even with duplicate titles or renamed labels.
`id_for_key` exposes that opaque model ID; controller callbacks carry the
domain key `K`. Removing and reinserting a key creates a new gesture
identity, so a pending click cannot close the replacement.

## Mouse and keyboard

| Input | Behavior |
|---|---|
| Left body click | Select on release; a drag does not activate first |
| Left close hit | Request close on release over the same live close target |
| Middle body or close hit | Request close without selecting or taking pane focus |
| Right body or close hit | Context request for that tab, without selecting, closing or taking pane focus |
| Other mouse buttons | No primary activation/close |
| Drag body, after 5 px | Show insertion point; release requests a keyed move, preserving selection |
| Escape / drop outside strip | Cancel the gesture |
| Left/Right/Home/End | Select a tab when the strip has focus |
| Alt+Left/Right | Move only the keyboard cursor |
| Mod+Shift+Left/Right | Request reorder of the cursor tab, without selecting it |
| Enter/Space | Explicitly activate the cursor tab |
| Delete | Request close of the cursor tab |
| Menu / Shift+F10 | Context request for the cursor tab |
| Ctrl+Tab/Ctrl+Shift+Tab, Ctrl+PageDown/PageUp | Existing window-wide next/previous tab shortcuts |

`focus_key(&key)` reveals and focuses a tab's keyboard cursor without
selecting it. Horizontal direction is logical: RTL mirrors pointer hits,
drop indicators and Left/Right. Scroll arrows respond only to the primary
mouse button. When using the component, include its enabled actions in the
app's F1 shortcut help.

Activation reveals its target and repaints even when it is already the
selected key. For example, `focus_key(&last)` can scroll the active first
tab out of view without selecting; Home brings that first tab back without
emitting a redundant selection change.

Ctrl+Shift+PageUp/PageDown are left to the app's controller (for example,
Files' Move commands); they never fall through to next/previous selection.

`TabReorder<K> { key, before }` requests insertion immediately before a
stable key; `None` means the end. `apply_to(&model)` resolves current
positions and ignores a missing source/target or self-target. An obsolete
target is never silently converted to append.

## Geometry for fixtures and controllers

All coordinates below are **window logical coordinates**, matching FLTK
`event_x`/`event_y`, not desktop coordinates. Add the window origin only
when driving an external native-input tool.

| API | Result |
|---|---|
| `rect_for_key(&key)` | Visible `(x, width)`; y/height are `tabs.y()`/`tabs.h()` |
| `close_rect_for_key(&key)` | Visible `(x, y, width, height)` of the close **hit area**, including clickable padding |
| `hit_test(x, y)` | `TabHit { key, part: TabPart::Body/Close }` |

These use the widget's own geometry, including scrolling, clipping and
RTL. Missing, hidden, zero-area, disposed or completely scrolled-out
targets return `None`; close geometry also returns `None` without a close
handler. Hit testing returns `None` for scroll arrows, gaps and outside
points. Middle-click routing uses the returned key for either tab part.
Context requests provide the same coordinate system in `request.x/y`.

## Transparent drawing in plain FLTK

TabBar uses `NoBox`: its parent owns the background. `AppWindow` already
restores that background on partial redraws. In a plain FLTK window, put
the strip in a background-owning group and draw its children after the
background on **every** group draw, including animation timer frames:

```rust
use fltk::{group::Group, prelude::*};
use fltk_omarchy::{paint, theme, widgets::TabBar, State};

let mut host = Group::new(8, 8, 300, 32, None);
let active = State::new(0usize);
let mut tabs = TabBar::new(&["First", "Second", "Third"], &active);
tabs.resize(8, 8, 300, 32);
host.end();
host.draw(|g| {
    theme::with(|t| paint::fill(g.x(), g.y(), g.w(), g.h(), 0,
                              t.palette.background));
    for i in 0..g.children() {
        if let Some(mut child) = g.child(i) {
            if child.visible() { g.draw_child(&mut child); }
        }
    }
});
host.super_draw(false);
```

Use the owning pane's actual theme surface in place of `background` when
appropriate. This restores transparent fills without painting across
neighboring panes. Disable the group's default pre-draw as above to avoid
drawing its children once before clearing the background. The strip needs
no `on_render` callback. Do not replace its own `draw` callback: that
would replace the toolkit renderer. Query the geometry APIs when needed;
there is no per-frame geometry notification.

## Validation

`tests/tabs_ui` covers mouse-button separation, duplicate titles, compound
pane/tab keys, focus preservation, updates during a press, removal and
reinsertion, pending model notifications, snapshots, cursor reorder,
cancellation, scrolling and RTL hit geometry. It uses the normal event
dispatcher and CPU snapshots.

The plain-FLTK host regression also reads the mapped window's actual RGB
buffer after hover enter/leave and start/middle/end timer frames, compares
each against a fresh full-background render, and checks that rendering
stops once motion settles. These are render checks using testkit input;
they are separate from the native XTest input check below.

For real X11 mouse/key events on a fresh private display:

```sh
cargo build --example tabs_input_check
script/tabs-input-check
```

The driver uses XTest, reads tab/close geometry from the actual widget and
checks right-click body/cross, real button 2, drag and keyboard reorder,
then explicit primary selection. A final overflow scenario scrolls that
active first key away with cursor-only focus, sends real Home, and verifies
that it is visible again with zero offset and no extra selection change.
It records requests and selection state
in `build/tabs-input-*`, never injects input into the user's display and
does not enable GPU rendering or effects.
