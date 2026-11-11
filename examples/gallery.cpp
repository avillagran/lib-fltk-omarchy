// Gallery: every widget of lib-fltk-omarchy in a Folio-like file manager
// layout, with fake data (no files are read or written).
//
//   gallery                 # interactive
//   gallery --demo=menu     # opens one component's state (for screenshots):
//                           # grid, list, columns, menu, dialog, prompt, help,
//                           # toast, tooltip, sidebar-focus
#include "fltk-omarchy/fo.h"

#include <FL/Fl.H>

#include <cstring>
#include <ctime>
#include <memory>
#include <string>
#include <vector>

namespace {

struct Entry {
  std::string name, icon, detail;
  bool dir;
};

std::vector<Entry> sample_entries() {
  std::vector<Entry> v = {
      {"Alfama", "folder", "2 items", true},
      {"Belém", "folder", "2 items", true},
      {"Sintra day trip", "folder", "3 items", true},
      {"azulejos.jpg", "file-image", "31.6 KB", false},
      {"fado-live.m4a", "file-music", "45.3 KB", false},
      {"fado-night.mp4", "file-video", "65.2 KB", false},
      {"harbour-dusk.jpg", "file-image", "27.1 KB", false},
      {"itinerary.pdf", "file-text", "44.6 KB", false},
      {"miradouro.jpg", "file-image", "30.0 KB", false},
      {"pasteis.jpg", "file-image", "32.6 KB", false},
      {"ponte.jpg", "file-image", "30.7 KB", false},
      {"tram-28.jpg", "file-image", "28.2 KB", false},
      {"trip-notes.md", "file-code", "717 B", false},
      {"a-very-long-file-name-from-the-camera-roll-2026-11-10-18-11-04.jpeg", "file-image", "4.2 MB", false},
  };
  // A big folder: the collection only draws what is visible.
  for (int i = 0; i < 2000; ++i) {
    char name[64];
    std::snprintf(name, sizeof name, "IMG_%04d.jpg", i + 1);
    v.push_back({name, "file-image", fo::i18n::format_size(uint64_t(1000000 + i * 3711)), false});
  }
  return v;
}

class Files : public fo::Collection {
public:
  Files(int x, int y, int w, int h) : Collection(x, y, w, h), entries(sample_entries()) {}
  std::vector<Entry> entries;
  int count() const override { return int(entries.size()); }
  std::string item_text(int i) const override { return entries[size_t(i)].name; }
  void draw_item(int i, int x, int y, int w, int h, const ItemState &s) override {
    const Entry &e = entries[size_t(i)];
    const fo::Theme &t = fo::theme();
    if (layout() == Layout::List)
      draw_row(x, y, w, h, s, e.icon, e.name, e.detail, e.dir ? t.accent() : fo::Rgb(), e.dir);
    else
      draw_tile(x, y, w, h, s, e.dir ? "" : e.icon, e.name, e.detail + " · 6:10 PM", kind_color(e), e.dir);
  }
  static fo::Rgb kind_color(const Entry &e) {
    const fo::Theme &t = fo::theme();
    if (e.icon == "file-image") return t.cyan();
    if (e.icon == "file-music") return t.magenta();
    if (e.icon == "file-video") return t.orange();
    if (e.icon == "file-text") return t.red();
    if (e.icon == "file-code") return t.blue();
    return t.secondary_foreground();
  }
};

// One folder of the columns view: a fake tree, the same at every depth.
class Folder : public fo::Collection {
public:
  Folder(std::string name, int depth) : Collection(0, 0, 10, 10), name(std::move(name)), depth(depth) {
    const char *dirs[] = {"Alfama", "Belém", "Sintra day trip", "Porto"};
    int nd = depth < 3 ? 4 - depth : 0;
    for (int i = 0; i < nd; ++i)
      entries.push_back({dirs[i], "folder", "", true});
    for (int i = 0; i < 6 + depth * 3; ++i) {
      char n[48];
      std::snprintf(n, sizeof n, "photo-%d-%02d.jpg", depth + 1, i + 1);
      entries.push_back({n, "file-image", "", false});
    }
  }
  std::string name;
  int depth;
  std::vector<Entry> entries;
  int count() const override { return int(entries.size()); }
  std::string item_text(int i) const override { return entries[size_t(i)].name; }
  void draw_item(int i, int x, int y, int w, int h, const ItemState &s) override {
    const Entry &e = entries[size_t(i)];
    draw_row(x, y, w, h, s, e.icon, e.name, "", e.dir ? fo::theme().accent() : Files::kind_color(e), e.dir);
  }
  bool dir_selected() const { return current() >= 0 && entries[size_t(current())].dir; }
};

// The right-hand details pane: plain FLTK group with toolkit widgets.
class Details : public Fl_Group {
public:
  Details(int x, int y, int w, int h) : Fl_Group(x, y, w, h) {
    box(FL_NO_BOX);
    field = new fo::TextField(x + 28, y + 330, w - 56, 38);
    field->placeholder("Filter this folder");
    open = new fo::Button(x + 28, y + h - 68, w - 56, 40, "Open");
    open->primary(true);
    plain = new fo::Button(x + 28, y + h - 118, w - 56, 36, "Show in terminal");
    end();
    resizable(nullptr);
  }
  void resize(int X, int Y, int W, int H) override {
    Fl_Widget::resize(X, Y, W, H);
    field->resize(X + 28, Y + 330, W - 56, 38);
    plain->resize(X + 28, Y + H - 118, W - 56, 36);
    open->resize(X + 28, Y + H - 68, W - 56, 40);
  }
  void draw() override {
    const fo::Theme &t = fo::theme();
    fo::paint::fill(x(), y(), w(), h(), t.background());
    int aw = w() - 56;
    fo::paint::round_fill(x() + 28, y() + 28, aw, aw * 3 / 4, std::min(10, t.radius()), t.lighter_background());
    fo::icon::draw("image", x() + 28 + aw / 2 - 24, y() + 28 + aw * 3 / 8 - 24, 48, t.cyan());
    int ty = y() + 40 + aw * 3 / 4;
    fo::text::draw(title, x() + 28, ty, aw, 24, t.bright_foreground(), fo::text::SEMIBOLD, fo::text::size::TITLE);
    fo::text::draw("JPEG image · 27.1 KB", x() + 28, ty + 24, aw, 18, t.secondary_foreground(), fo::text::REGULAR,
                   fo::text::size::LABEL);
    fo::text::draw("Modified", x() + 28, ty + 58, aw, 18, t.secondary_foreground(), fo::text::REGULAR,
                   fo::text::size::LABEL);
    fo::text::draw(fo::i18n::format_date(std::time(nullptr)) + " " + fo::i18n::format_time(std::time(nullptr)),
                   x() + 28, ty + 58, aw, 18, t.foreground(), fo::text::REGULAR, fo::text::size::LABEL, FL_ALIGN_RIGHT);
    draw_children();
  }
  std::string title = "harbour-dusk.jpg";
  fo::TextField *field;
  fo::Button *open, *plain;
};

std::vector<fo::HelpOverlay::Row> help_rows() {
  return {{"\xE2\x86\x90 \xE2\x86\x91 \xE2\x86\x92 \xE2\x86\x93", "Move the selection"},
          {"Shift+click / Shift+arrows", "Select a range"},
          {"Ctrl+click / Alt+click", "Add or remove one from the selection"},
          {"Drag on empty space", "Select with a rectangle"},
          {"Ctrl+A", "Select everything"},
          {"Tab", "Move between the sidebar and the files"},
          {"\xE2\x8F\x8E", "Open"},
          {"Ctrl+1 / 2 / 3", "Grid, list or columns"},
          {"Ctrl+L", "Focus the path"},
          {"Ctrl+I", "Show or hide the details pane"},
          {"F2", "Rename"},
          {"Delete", "Move to the trash (asks)"},
          {"Right-click / Menu", "Everything else"},
          {"Ctrl+T", "Show a toast"},
          {"?", "Show these shortcuts"},
          {"Q", "Close the window"}};
}

} // namespace

int main(int argc, char **argv) {
  std::string demo;
  for (int i = 1; i < argc; ++i)
    if (!std::strncmp(argv[i], "--demo=", 7))
      demo = argv[i] + 7;

  fo::InitOptions opts;
  opts.app_class = "fo-gallery";
  fo::init(opts);

  const int W = 1280, H = 800, TOP = 60, SIDE = 236;
  auto *win = new fo::Window(W, H, "lib-fltk-omarchy gallery");

  // Top bar: breadcrumbs and view buttons.
  auto *crumbs = new fo::Breadcrumbs(26, 0, W - 26 - 220, TOP);
  crumbs->segments({{"~", "/home/me"}, {"Pictures", "/home/me/Pictures"}, {"Lisbon 2026", "/home/me/Pictures/Lisbon"}});
  auto *b_view = new fo::IconButton(W - 200, 12, 36, 36, "grid", "Grid (Ctrl+1); click for list");
  auto *b_sort = new fo::IconButton(W - 160, 12, 36, 36, "arrow-down-a-z", "Sort by name");
  auto *b_hidden = new fo::IconButton(W - 120, 12, 36, 36, "eye", "Show hidden files (Ctrl+H)");
  auto *b_details = new fo::IconButton(W - 80, 12, 36, 36, "panel-right", "Details (Ctrl+I)");
  b_details->active_state(true);
  new fo::Rule(0, TOP - 1, W, 1);

  // Body: sidebar | files | details.
  auto *split = new fo::SplitPane(0, TOP, W, H - TOP);
  auto *sidebar = new fo::Sidebar(0, TOP, SIDE, H - TOP);
  auto *inner = new fo::SplitPane(SIDE + 1, TOP, W - SIDE - 1, H - TOP);
  auto *body = new Fl_Group(SIDE + 1, TOP, W - SIDE - 1 - 300, H - TOP);
  auto *files = new Files(SIDE + 1, TOP, W - SIDE - 1 - 300, H - TOP);
  auto *columns = new fo::ColumnView(SIDE + 1, TOP, W - SIDE - 1 - 300, H - TOP);
  columns->end();
  columns->hide();
  body->end();
  auto *details = new Details(W - 299, TOP, 299, H - TOP);
  inner->end();
  inner->min_sizes(300, 220);
  inner->set_split(W - SIDE - 1 - 300);
  split->end();
  split->min_sizes(160, 400);
  split->set_split(SIDE);
  win->end();
  win->resizable(split);

  sidebar->add_section("Home", "/home/me");
  for (auto r : std::vector<std::pair<const char *, const char *>>{
           {"Documents", "file-text"}, {"Downloads", "download"}, {"Music", "music"},
           {"Pictures", "image"}, {"Projects", "folder"}, {"Videos", "video"}, {"Work", "folder"}})
    sidebar->add_row({r.first, r.second, std::string("/home/me/") + r.first});
  sidebar->add_section("Views");
  sidebar->add_row({"Recents", "clock", "recent://"});
  sidebar->add_row({"Trash", "trash-2", "trash://"});
  sidebar->add_section("Devices");
  fo::SidebarRow usb{"KINGSTON", "usb", "/run/media/me/KINGSTON"};
  usb.ejectable = true;
  sidebar->add_row(usb);
  fo::SidebarRow cloud{"Dropbox", "cloud", "/home/me/Dropbox", "syncing", fo::theme().yellow()};
  sidebar->add_row(cloud);
  sidebar->add_section("Tailnet");
  sidebar->add_row({"laptop", "laptop", "sftp://laptop"});
  sidebar->add_row({"studio", "server", "sftp://studio"});
  sidebar->set_active("/home/me/Pictures");

  files->layout(fo::Collection::Layout::Grid);
  files->reload(true);
  files->select(6, true);
  files->set_current(6, true);

  // The columns view: picking a folder shows what is inside in the next
  // column; Right on a folder goes into it, Left back (ColumnView).
  auto wire_column = std::make_shared<std::function<void(Folder *)>>();
  auto open_inside = [=](Folder *f) {
    int at = columns->index_of(f);
    columns->truncate(at + 1);
    if (f->dir_selected()) {
      auto *next = new Folder(f->entries[size_t(f->current())].name, f->depth + 1);
      next->reload(true);
      next->set_current(-1);
      (*wire_column)(next);
      columns->add_column(next);
    }
  };
  *wire_column = [=](Folder *f) {
    f->on_selection_change([=] { open_inside(f); });
    f->on_activate([=](int i) { fo::Toast::show(win, "Open " + f->entries[size_t(i)].name); });
  };
  {
    auto *root = new Folder("Lisbon 2026", 0);
    root->reload(true);
    (*wire_column)(root);
    columns->add_column(root);
    root->set_current(0);
  }
  columns->on_step([=](int from, int dir) {
    // Right on a folder enters the column showing it.
    auto *f = static_cast<Folder *>(columns->column(from));
    if (dir > 0 && f->dir_selected() && from + 1 < columns->columns()) {
      auto *next = static_cast<Folder *>(columns->column(from + 1));
      columns->set_active(from + 1);
      if (next->current() < 0 && next->count() > 0)
        next->set_current(0);
      return true;
    }
    return dir > 0; // nothing to enter
  });

  // Behavior.
  auto set_view = [=](int view) { // 0 grid, 1 list, 2 columns
    if (view == 2) {
      files->hide();
      columns->show();
      columns->set_active(columns->columns() - 1);
      b_view->icon("columns-3");
    } else {
      columns->hide();
      files->show();
      files->layout(view == 0 ? fo::Collection::Layout::Grid : fo::Collection::Layout::List);
      files->take_focus();
      b_view->icon(view == 0 ? "grid" : "list");
    }
  };
  b_view->on_click([=] {
    set_view(columns->visible() ? 0 : files->layout() == fo::Collection::Layout::Grid ? 1 : 2);
  });
  b_hidden->on_click([=] { b_hidden->active_state(!b_hidden->active_state()); });
  b_details->on_click([=] {
    b_details->active_state(!b_details->active_state());
    inner->collapse_second(!b_details->active_state());
  });
  auto sort_menu = [=] {
    fo::PopupMenu::open_below(win, b_sort->x(), b_sort->y(), b_sort->w(), b_sort->h(),
                              {fo::MenuItem("Name", "S").check(),
                               fo::MenuItem("Kind"),
                               fo::MenuItem("Newest first"),
                               fo::MenuItem("Oldest first"),
                               fo::MenuItem("Size")},
                              true);
  };
  b_sort->on_click(sort_menu);
  crumbs->on_navigate([=](int i) { fo::Toast::show(win, "Go to " + crumbs->segments()[size_t(i)].path); });
  sidebar->on_activate([=](const fo::SidebarRow &r) {
    sidebar->set_active(r.id);
    fo::Toast::show(win, "Open " + r.label);
  });
  sidebar->on_eject([=](const fo::SidebarRow &r) { fo::Toast::show(win, "Ejected " + r.label); });
  sidebar->on_finished([=] { files->take_focus(); });
  files->on_selection_change([=] {
    int c = files->current();
    if (c >= 0) {
      details->title = files->entries[size_t(c)].name;
      details->redraw();
    }
  });
  files->on_activate([=](int i) { fo::Toast::show(win, "Open " + files->entries[size_t(i)].name); });

  auto rename = [=] {
    int c = files->current();
    if (c < 0)
      return;
    std::string name = files->entries[size_t(c)].name;
    size_t dot = name.rfind('.');
    auto *d = fo::Dialog::prompt(win, "Rename", "", name, fo::tr("Rename"),
                                 [=](bool ok, const std::string &text) {
                                   if (ok) {
                                     files->entries[size_t(c)].name = text;
                                     files->redraw();
                                   }
                                 },
                                 0, dot == std::string::npos || dot == 0 ? -1 : int(dot));
    d->on_validate([d](const std::string &s) {
      if (s.empty() || s.find('/') != std::string::npos) {
        d->set_error("Names can't be empty or contain /");
        return false;
      }
      return true;
    });
  };
  auto confirm_delete = [=] {
    int n = files->selection_count();
    fo::Dialog::confirm(win, "Delete for good?",
                        fo::trn("%n items", n) + " will be deleted permanently. This can't be undone.",
                        fo::tr("Delete"), [=](bool yes) {
                          if (yes)
                            fo::Toast::show(win, "Nothing was deleted: this is a gallery", 3, fo::Toast::Error);
                        },
                        true);
  };
  auto context = [=](int index, int x, int y) {
    std::vector<fo::MenuItem> items = {
        fo::MenuItem("Open", "\xE2\x8F\x8E", [=] { fo::Toast::show(win, "Open"); }).with_icon("external-link"),
        fo::MenuItem("Open with", "", nullptr,
                     {fo::MenuItem("Image Viewer"), fo::MenuItem("GIMP"), fo::MenuItem::separator(),
                      fo::MenuItem("Other application…")})
            .with_icon("app-window"),
        fo::MenuItem::separator(),
        fo::MenuItem("Copy", "Ctrl+C").with_icon("copy"),
        fo::MenuItem("Cut", "Ctrl+X").with_icon("scissors"),
        fo::MenuItem("Paste", "Ctrl+V").with_icon("clipboard").disabled(),
        fo::MenuItem("Copy path", "Ctrl+Shift+C").with_icon("link"),
        fo::MenuItem::separator(),
        fo::MenuItem("Rename", "F2", rename).with_icon("pencil"),
        fo::MenuItem("Move to trash", "Delete", confirm_delete).with_icon("trash-2"),
    };
    if (index < 0)
      items = {fo::MenuItem("New folder", "Ctrl+Shift+N").with_icon("folder-plus"),
               fo::MenuItem("Open terminal here").with_icon("terminal"), fo::MenuItem::separator(),
               fo::MenuItem("Show hidden files", "Ctrl+H").check(b_hidden->active_state())};
    fo::PopupMenu::open_at(win, x, y, items);
  };
  files->on_context(context);
  files->on_key([=](int k) {
    if (k == FL_F + 2) { rename(); return true; }
    if (k == FL_Delete) { confirm_delete(); return true; }
    if (k == FL_Menu) {
      int x, y, w, h;
      if (files->item_rect(files->current(), x, y, w, h))
        context(files->current(), x + w / 2, y + h / 2);
      return true;
    }
    return false;
  });

  win->shortcut("Ctrl+1", [=] { set_view(0); });
  win->shortcut("Ctrl+2", [=] { set_view(1); });
  win->shortcut("Ctrl+3", [=] { set_view(2); });
  win->shortcut("Ctrl+L", [=] { crumbs->take_focus(); });
  win->shortcut("Ctrl+I", [=] { b_details->do_callback(); });
  win->shortcut("Ctrl+T", [=] { fo::Toast::show(win, "Copied 3 items"); });
  win->shortcut("?", [=] { fo::HelpOverlay::show(win, help_rows()); });
  win->shortcut("q", [=] { win->hide(); });

  if (!demo.empty())
    win->position(0, 0); // screenshots crop from the corner
  win->show(1, argv);
  files->take_focus();

  // Demo states for screenshots.
  struct Demo {
    std::string name;
    fo::Window *win;
    Files *files;
    fo::Sidebar *sidebar;
    fo::IconButton *sort;
    std::function<void(int, int, int)> context;
    std::function<void()> rename, del, sort_menu;
    std::function<void(int)> view;
    fo::ColumnView *columns;
  };
  static Demo d{demo, win, files, sidebar, b_sort, context, rename, confirm_delete, sort_menu, set_view, columns};
  if (!demo.empty())
    Fl::add_timeout(0.4, [](void *) {
      const std::string &n = d.name;
      if (n == "list")
        d.files->layout(fo::Collection::Layout::List);
      else if (n == "columns") {
        d.view(2);
        // Two levels down, the keyboard in the deepest folder.
        auto *c1 = static_cast<fo::Collection *>(d.columns->column(1));
        if (c1)
          c1->set_current(1);
        auto *c2 = static_cast<fo::Collection *>(d.columns->column(2));
        if (c2) {
          c2->set_current(4);
          d.columns->set_active(2);
        }
        fo::set_focus_visible(true);
      }
      else if (n == "menu")
        d.context(6, 760, 360);
      else if (n == "submenu") {
        d.context(6, 760, 360);
      } else if (n == "sort")
        d.sort_menu();
      else if (n == "dialog")
        d.del();
      else if (n == "prompt")
        d.rename();
      else if (n == "help")
        fo::HelpOverlay::show(d.win, help_rows());
      else if (n == "toast")
        fo::Toast::show(d.win, "Copied 3 items", 30);
      else if (n == "tooltip")
        fo::Tooltip::open_for(d.sort, "Sort by name");
      else if (n == "sidebar-focus") {
        fo::set_focus_visible(true);
        d.sidebar->take_focus();
        d.sidebar->set_cursor(9);
      } else if (n == "band") {
        d.files->layout(fo::Collection::Layout::List);
        d.files->select_all();
      }
      d.win->redraw();
    });
  return Fl::run();
}
