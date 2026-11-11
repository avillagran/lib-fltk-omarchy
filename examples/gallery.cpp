// Gallery: every widget of lib-fltk-omarchy in the layout of Files (the
// file manager whose look the toolkit carries), with fake data: no files
// are read or written.
//
//   gallery                 # interactive
//   gallery --demo=NAME     # opens one component's state (for screenshots):
//                           # groups, list, grid, columns, rail, menu,
//                           # actions, dialog, prompt, help, toast, tooltip,
//                           # sidebar-focus
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
      {"Alfama", "folder", "\xE2\x88\x91 4.2 kB", true},
      {"Belém", "folder", "\xE2\x88\x91 3.1 MB", true},
      {"Sintra day trip", "folder", "\xE2\x88\x91 30.3 kB", true},
      {"azulejos.jpg", "file-image", "31.6 kB", false},
      {"fado-live.m4a", "file-music", "45.3 kB", false},
      {"fado-night.mp4", "file-video", "65.2 kB", false},
      {"harbour-dusk.jpg", "file-image", "27.1 kB", false},
      {"itinerary.pdf", "file-text", "44.6 kB", false},
      {"miradouro.jpg", "file-image", "30.0 kB", false},
      {"pasteis.jpg", "file-image", "32.6 kB", false},
      {"ponte.jpg", "file-image", "30.7 kB", false},
      {"tram-28.jpg", "file-image", "28.2 kB", false},
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

// Files' Groups: folder cards under "FOLDERS · 3", then the files as rows
// under "TODAY · n"; List and Grid draw the same entries.
class Files : public fo::Collection {
public:
  Files(int x, int y, int w, int h) : Collection(x, y, w, h), entries(sample_entries()) {
    always_ring(true);
    groups();
  }
  std::vector<Entry> entries;
  void groups() {
    Band cards;
    cards.count = 3, cards.min_width = 240, cards.item_h = 74, cards.gap = 8, cards.header_h = 28, cards.after = 6;
    Band rows;
    rows.count = int(entries.size()) - 3, rows.item_h = 38, rows.header_h = 26, rows.after = 10;
    bands({cards, rows});
    layout(Layout::Bands);
    padding(0);
    ring_radius(11);
  }
  int count() const override { return int(entries.size()); }
  std::string item_text(int i) const override { return entries[size_t(i)].name; }
  void draw_band_header(int band, int x, int y, int w, int h) override {
    std::string label = band == 0 ? "FOLDERS \xC2\xB7 3" : "TODAY \xC2\xB7 " + std::to_string(count() - 3);
    fo::draw_section_label(label, x, y, w, 24);
    if (band == 0) {
      int bx = x + int(fo::text::width(label, fo::text::REGULAR, 10)) + 14;
      fo::paint::round_fill(bx, y, 28, 24, fo::theme().radius() > 0 ? 4 : 0, fo::theme().selection());
      fo::icon::draw("folder-plus", bx + 7, y + 5, 14, fo::theme().accent());
    }
    (void)h;
  }
  void draw_item(int i, int x, int y, int w, int h, const ItemState &s) override {
    const Entry &e = entries[size_t(i)];
    const fo::Theme &t = fo::theme();
    if (layout() == Layout::Grid) {
      draw_tile(x, y, w, h, s, e.icon, e.name, e.detail, e.dir ? t.blue() : kind_color(e), e.dir);
      return;
    }
    if (layout() == Layout::Bands && i < 3) {
      // A folder card.
      int r = t.radius() > 0 ? 11 : 0;
      fo::paint::round_fill(x, y, w, h, r, s.selected ? t.selection() : t.card());
      if (s.current)
        fo::paint::fill(x + 4, y + 8, 2, h - 16, t.accent());
      fo::icon::draw("folder", x + 12, y + 22, 25, t.blue());
      fo::text::draw(e.name, x + 54, y + 10, w - 64, 24, t.bright_foreground(), fo::text::REGULAR, 13);
      fo::text::draw(e.detail, x + 54, y + 34, w - 64, 18, t.dim(), fo::text::REGULAR, 11);
      fo::text::draw("now", x + 54, y + 51, w - 64, 16, t.dim(), fo::text::REGULAR, 10);
      return;
    }
    draw_row(x, y, w, h, s, e.icon, e.name, e.detail, e.dir ? t.blue() : kind_color(e), false);
  }
  static fo::Rgb kind_color(const Entry &e) {
    const fo::Theme &t = fo::theme();
    if (e.icon == "file-image") return t.magenta();
    if (e.icon == "file-music") return t.cyan();
    if (e.icon == "file-video") return t.orange();
    if (e.icon == "file-text") return t.red();
    if (e.icon == "file-code") return t.green();
    return t.dim();
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
    draw_row(x, y, w, h, s, e.icon, e.name, "", e.dir ? fo::theme().blue() : Files::kind_color(e), e.dir);
  }
  bool dir_selected() const { return current() >= 0 && entries[size_t(current())].dir; }
};

// Files' inspector: a preview tile, the name, kind and facts, and the
// action buttons (fo::draw_action_button).
class Inspector : public Fl_Widget {
public:
  Inspector(int x, int y, int w, int h) : Fl_Widget(x, y, w, h) {}
  void draw() override {
    const fo::Theme &t = fo::theme();
    fo::paint::fill(x(), y(), w(), h(), t.background());
    fo::paint::vline(x(), y(), h(), t.card());
    fo::draw_section_label("Inspector", x() + 16, y() + 12, w() - 32, 22);
    int iw = w() - 32, r = t.radius() > 0 ? 7 : 0;
    fo::paint::round_fill(x() + 16, y() + 50, iw, 98, r, t.card());
    fo::icon::draw("image", x() + 16 + iw / 2 - 16, y() + 50 + 33, 32, t.magenta());
    int ty = y() + 160;
    fo::text::draw(title, x() + 16, ty, iw, 36, t.bright_foreground(), fo::text::REGULAR, 13);
    fo::text::draw("image/jpeg", x() + 16, ty + 44, iw, 22, t.dim(), fo::text::REGULAR, 11);
    fo::text::draw("Size      27.1 kB", x() + 16, ty + 72, iw, 22, t.foreground(), fo::text::REGULAR, 12);
    fo::text::draw("now", x() + 16, ty + 98, iw, 22, t.foreground(), fo::text::REGULAR, 12);
    int b = y() + h();
    fo::draw_action_button(x() + 16, b - 136, iw, 30, "Rename \xC2\xB7 F2", false, false);
    int half = (iw - 8) / 2;
    fo::draw_action_button(x() + 16, b - 94, half, 34, "Open", true, false);
    fo::draw_action_button(x() + 24 + half, b - 94, half, 34, "Preview", false, false);
    fo::draw_action_button(x() + 16, b - 52, 60, 34, "Path", false, false);
    fo::draw_action_button(x() + 80, b - 52, 68, 34, "F5", false, false);
    fo::draw_action_button(x() + 152, b - 52, 68, 34, "F6", false, false);
  }
  std::string title = "harbour-dusk.jpg";
};

std::vector<fo::HelpOverlay::Row> help_rows() {
  return {{"Type", "Filter this folder (Esc clears)"},
          {"Ctrl+L", "Edit the path; >command or :place opens a catalog"},
          {"Ctrl+Shift+P", "Commands"},
          {"Enter", "Open"},
          {"Backspace / Alt+Up", "Go to the parent folder"},
          {"Ctrl+1 \xE2\x80\xA6 5", "Groups, List, Columns, Space, Grid"},
          {"Ctrl+A / Ctrl+I", "Select all / invert the selection"},
          {"Insert", "Mark and move on"},
          {"F2", "Rename"},
          {"F5 / F6", "Copy / move"},
          {"F7", "New folder"},
          {"Delete", "Move to the trash"},
          {"F9 / Tab", "Dual pane / switch pane"},
          {"F10 / Ctrl+,", "Actions / Preferences"},
          {"Ctrl+T", "Show a toast"},
          {"F1", "Show these shortcuts"}};
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

  // Files' metrics: header 58, tabs 32, rail 52, inspector 240, status 32.
  const int W = 1100, H = 700, TOP = 90, RAIL = 52, INSP = 240, STATUS = 32;
  auto *win = new fo::Window(W, H, "lib-fltk-omarchy gallery");

  auto *nav = new fo::ToolBar(12, 13, 102, 32);
  nav->plain(true);
  nav->metrics(30, 6);
  auto *crumbs = new fo::Breadcrumbs(136, 12, 400, 34);
  crumbs->segments({{"/", "/"}, {"home", "/home"}, {"Home", "/home/me"}, {"Pictures", "/home/me/Pictures"},
                    {"Lisbon 2026", "/home/me/Pictures/Lisbon"}});
  auto *tools = new fo::ToolBar(W - 412, 10, 400, 36);
  auto *tabs = new fo::TabStrip(0, 58, W, 32);
  tabs->tabs({"Lisbon 2026", "Downloads"}, 0);
  auto *rail = new fo::Sidebar(0, TOP, RAIL, H - TOP - STATUS);
  rail->rail(true);
  auto *header = new fo::SortHeader(RAIL + 16, TOP + 4, W - RAIL - INSP - 32, 30);
  header->hide();
  auto *files = new Files(RAIL + 16, TOP + 4, W - RAIL - INSP - 32, H - TOP - STATUS - 12);
  auto *columns = new fo::ColumnView(RAIL + 16, TOP + 4, W - RAIL - INSP - 32, H - TOP - STATUS - 12);
  columns->end();
  columns->hide();
  auto *inspector = new Inspector(W - INSP, TOP, INSP, H - TOP - STATUS);
  auto *status = new fo::StatusBar(0, H - STATUS, W, STATUS);
  status->left("1 marked \xC2\xB7 2014 items");
  status->hint("Ctrl+L: type >command or :place and press Enter to open the catalog");
  status->right("ext4 \xC2\xB7 640 GB free");
  status->badge("PSS 10.0 MiB", fo::theme().green());
  win->end();

  rail->add_section("");
  rail->add_row({"Recents", "recent", "recent://"});
  rail->add_row({"Lisbon", "star", "/home/me/Pictures/Lisbon"});
  rail->add_section("Places");
  for (auto r : std::vector<std::pair<const char *, const char *>>{{"Home", "home"},
                                                                  {"Downloads", "downloads"},
                                                                  {"Documents", "documents"},
                                                                  {"Pictures", "pictures"},
                                                                  {"Music", "music"},
                                                                  {"Videos", "videos"}})
    rail->add_row({r.first, r.second, std::string("/home/me/") + r.first});
  rail->add_section("Drives");
  fo::SidebarRow usb{"KINGSTON", "usb", "/run/media/me/KINGSTON"};
  usb.ejectable = true;
  rail->add_row(usb);
  rail->add_section("Network");
  rail->add_row({"laptop", "laptop", "sftp://laptop"});
  fo::SidebarRow trash{"Trash", "trash", "trash://"};
  trash.footer = true;
  rail->add_row(trash);
  rail->set_active("/home/me/Pictures");

  files->reload(true);
  files->set_current(4, true);
  header->columns({{"Name", "sort", 48, 500}, {"Size", "database", 560, 104, true}});

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
    auto *f = static_cast<Folder *>(columns->column(from));
    if (dir > 0 && f->dir_selected() && from + 1 < columns->columns()) {
      auto *next = static_cast<Folder *>(columns->column(from + 1));
      columns->set_active(from + 1);
      if (next->current() < 0 && next->count() > 0)
        next->set_current(0);
      return true;
    }
    return dir > 0;
  });

  auto layout = std::make_shared<std::function<void()>>();
  auto view = std::make_shared<int>(0); // 0 groups, 1 list, 2 columns, 4 grid
  auto set_view = [=](int v) {
    *view = v;
    int cx = rail->desired_width() + 16, cw = W - cx - INSP - 16, top = TOP + 4;
    if (v == 2) {
      files->hide();
      header->hide();
      columns->show();
      columns->set_active(columns->columns() - 1);
    } else {
      columns->hide();
      files->show();
      if (v == 0) {
        files->groups();
        header->hide();
        files->resize(cx, top, cw, H - top - STATUS - 8);
      } else if (v == 1) {
        files->layout(fo::Collection::Layout::List);
        files->row_height(38);
        files->ring_radius(9);
        header->show();
        header->resize(cx, top, cw, 30);
        files->resize(cx, top + 30, cw, H - top - 30 - STATUS - 8);
      } else {
        files->layout(fo::Collection::Layout::Grid);
        files->tile_size(156, 132);
        files->ring_radius(8);
        header->hide();
        files->resize(cx, top, cw, H - top - STATUS - 8);
      }
      files->take_focus();
    }
    (*layout)();
  };
  auto actions = [=] {
    int x, y, w, h;
    tools->item_rect(int(tools->items().size()) - 2, x, y, w, h);
    fo::PopupMenu::open_below(
        win, x, y, w, h,
        {fo::MenuItem("Create and organize", "", nullptr,
                      {fo::MenuItem("Create file", "Ctrl+N").with_icon("file"),
                       fo::MenuItem("Create folder", "F7").with_icon("folder"),
                       fo::MenuItem("Rename", "F2").with_icon("edit"),
                       fo::MenuItem("Send to Trash", "Delete").with_icon("trash")})
             .with_icon("folder-plus"),
         fo::MenuItem("Clipboard", "", nullptr,
                      {fo::MenuItem("Copy", "Ctrl+C").with_icon("copy"), fo::MenuItem("Cut", "Ctrl+X").with_icon("cut"),
                       fo::MenuItem("Paste", "Ctrl+V").with_icon("paste").disabled()})
             .with_icon("paste"),
         fo::MenuItem("Selection", "", nullptr,
                      {fo::MenuItem("Select all", "Ctrl+A"), fo::MenuItem("Invert selection", "Ctrl+I")})
             .with_icon("select"),
         fo::MenuItem("Search and open", "", nullptr, {fo::MenuItem("Search in subfolders", "Ctrl+Shift+F")})
             .with_icon("search"),
         fo::MenuItem("Space and sizes", "", nullptr, {fo::MenuItem("Calculate size", "Ctrl+Space")})
             .with_icon("database"),
         fo::MenuItem("Tools", "", nullptr, {fo::MenuItem("Commands", "Ctrl+Shift+P")}).with_icon("terminal"),
         fo::MenuItem("History and results", "", nullptr, {fo::MenuItem("Undo", "Ctrl+Z")}).with_icon("history")},
        true);
  };
  auto tool_items = [=] {
    int v = *view;
    tools->items({{"search", "Search", "Ctrl+F", false, true, [] {}},
                  {"pencil", "Edit path", "Ctrl+L", false, true, [] {}},
                  {"groups", "Groups", "Ctrl+1", v == 0, true, [=] { set_view(0); }},
                  {"list", "List", "Ctrl+2", v == 1, true, [=] { set_view(1); }},
                  {"grid", "Grid", "Ctrl+5", v == 4, true, [=] { set_view(4); }},
                  {"space", "Space", "Ctrl+4", false, true, [] {}},
                  {"columns", "Columns", "Ctrl+3", v == 2, true, [=] { set_view(2); }},
                  {"info", "Inspector", "Ctrl+Alt+I", true, true, [] {}},
                  {"actions", "Actions", "F10", false, true, actions},
                  {"settings", "Preferences", "Ctrl+,", false, true, [] {}}});
  };
  *layout = [=] {
    tool_items();
    int rw = rail->desired_width();
    rail->resize(0, TOP, rw, H - TOP - STATUS);
    int pw = tools->preferred_width();
    tools->resize(W - 12 - pw, 10, pw, 36);
    crumbs->resize(136, 12, std::max(1, W - 12 - pw - 14 - 156), 34);
    int cx = rw + 16, cw = W - cx - INSP - 16;
    if (*view == 2)
      columns->resize(cx, TOP + 4, cw, H - TOP - STATUS - 12);
    else if (*view == 1) {
      header->resize(cx, TOP + 4, cw, 30);
      files->resize(cx, TOP + 34, cw, H - TOP - 30 - STATUS - 12);
    } else
      files->resize(cx, TOP + 4, cw, H - TOP - STATUS - 12);
    win->redraw();
  };
  tools->on_relayout([=] { (*layout)(); });
  rail->on_width([=] { (*layout)(); });
  nav->items({{"arrow-left", "Back", "Alt+Left", false, true, [=] { fo::Toast::show(win, "Back"); }},
              {"arrow-right", "Forward", "Alt+Right", false, false, nullptr},
              {"arrow-up", "Up", "Backspace", false, true, [=] { fo::Toast::show(win, "Up"); }}});
  (*layout)();

  crumbs->on_navigate([=](int i) { fo::Toast::show(win, "Go to " + crumbs->segments()[size_t(i)].path); });
  tabs->on_select([=](int i) { tabs->tabs({"Lisbon 2026", "Downloads"}, i); });
  tabs->on_new([=] { fo::Toast::show(win, "Tab created"); });
  rail->on_activate([=](const fo::SidebarRow &r) {
    rail->set_active(r.id);
    fo::Toast::show(win, "Open " + r.label);
  });
  rail->on_eject([=](const fo::SidebarRow &r) { fo::Toast::show(win, "Ejected " + r.label); });
  rail->on_finished([=] { files->take_focus(); });
  files->on_selection_change([=] {
    int c = files->current();
    if (c >= 0) {
      inspector->title = files->entries[size_t(c)].name;
      inspector->redraw();
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
        fo::MenuItem("Open focused item", "Enter", [=] { fo::Toast::show(win, "Open"); }).with_icon("open"),
        fo::MenuItem("Open with\xE2\x80\xA6", "Ctrl+Shift+O", nullptr,
                     {fo::MenuItem("Image Viewer"), fo::MenuItem("GIMP"), fo::MenuItem::separator(),
                      fo::MenuItem("Other application\xE2\x80\xA6")})
            .with_icon("external-link"),
        fo::MenuItem("Preview", "Space").with_icon("eye"),
        fo::MenuItem::separator(),
        fo::MenuItem("Cut", "Ctrl+X").with_icon("cut"),
        fo::MenuItem("Copy", "Ctrl+C").with_icon("copy"),
        fo::MenuItem("Paste", "Ctrl+V").with_icon("paste").disabled(),
        fo::MenuItem::separator(),
        fo::MenuItem("Rename", "F2", rename).with_icon("edit"),
        fo::MenuItem("Send to Trash", "Delete", confirm_delete).with_icon("trash"),
        fo::MenuItem::separator(),
        fo::MenuItem("Terminal here", "Ctrl+.").with_icon("terminal"),
        fo::MenuItem("Copy path", "Ctrl+Alt+C").with_icon("link"),
    };
    if (index < 0)
      items = {fo::MenuItem("Create folder", "F7").with_icon("folder-plus"),
               fo::MenuItem("Terminal here", "Ctrl+.").with_icon("terminal"), fo::MenuItem::separator(),
               fo::MenuItem("Show hidden files", "Ctrl+H").check(false)};
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
  win->shortcut("Ctrl+5", [=] { set_view(4); });
  win->shortcut("Ctrl+L", [=] { crumbs->take_focus(); });
  win->shortcut("Ctrl+T", [=] { fo::Toast::show(win, "Copied 3 items"); });
  win->shortcut("F10", actions);
  win->shortcut("F1", [=] { fo::HelpOverlay::show(win, help_rows()); });
  win->shortcut("Ctrl+Q", [=] { win->hide(); });

  if (!demo.empty())
    win->position(0, 0); // screenshots crop from the corner
  win->show(1, argv);
  files->take_focus();

  // Demo states for screenshots.
  struct Demo {
    std::string name;
    fo::Window *win;
    Files *files;
    fo::Sidebar *rail;
    fo::ToolBar *tools;
    std::function<void(int, int, int)> context;
    std::function<void()> rename, del, actions;
    std::function<void(int)> view;
    fo::ColumnView *columns;
  };
  static Demo d{demo, win, files, rail, tools, context, rename, confirm_delete, actions, set_view, columns};
  if (!demo.empty())
    Fl::add_timeout(0.4, [](void *) {
      const std::string &n = d.name;
      if (n == "list")
        d.view(1);
      else if (n == "grid")
        d.view(4);
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
      } else if (n == "rail") {
        d.rail->pinned(true);
      } else if (n == "menu")
        d.context(6, 560, 360);
      else if (n == "actions")
        d.actions();
      else if (n == "dialog")
        d.del();
      else if (n == "prompt")
        d.rename();
      else if (n == "help")
        fo::HelpOverlay::show(d.win, help_rows());
      else if (n == "toast")
        fo::Toast::show(d.win, "Copied 3 items", 30);
      else if (n == "tooltip") {
        int x, y, w, h;
        d.tools->item_rect(int(d.tools->items().size()) - 2, x, y, w, h);
        fo::Tooltip::open_at(d.win, x, y, w, h, "F10");
      } else if (n == "sidebar-focus") {
        fo::set_focus_visible(true);
        d.rail->take_focus();
        d.rail->set_cursor(4);
      }
      d.win->redraw();
    });
  return Fl::run();
}
