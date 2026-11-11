#include "fltk-omarchy/dialog.h"

#include "fltk-omarchy/i18n.h"
#include "fltk-omarchy/paint.h"
#include "fltk-omarchy/text.h"
#include "fltk-omarchy/widgets.h"

#include <FL/Fl.H>
#include <FL/fl_draw.H>

#include <algorithm>
#include <memory>

namespace fo {

namespace {
const double kScrim = 0.55;
const int kPad = 28, kTitleH = 22, kLineH = 19, kFieldH = 38, kButtonH = 36, kGap = 16;

void draw_card(int x, int y, int w, int h, Rgb edge) {
  const Theme &t = theme();
  paint::round_fill(x, y, w, h, t.radius(), t.background());
  paint::round_frame(x, y, w, h, t.radius(), 1, edge);
}
} // namespace

// --- Dialog -----------------------------------------------------------------

Dialog *Dialog::confirm(Window *win, const std::string &title, const std::string &message,
                        const std::string &action, std::function<void(bool)> done, bool danger) {
  DialogSpec s;
  s.title = title;
  s.message = message;
  s.danger = danger;
  s.buttons = {tr("Cancel"), action};
  s.focus_field = -1;
  return ask(win, std::move(s), [done](const DialogAnswer &a) {
    if (done)
      done(a.button == 1);
  });
}

Dialog *Dialog::prompt(Window *win, const std::string &title, const std::string &message, const std::string &initial,
                       const std::string &action, std::function<void(bool, const std::string &)> done,
                       int select_from, int select_to) {
  DialogSpec s;
  s.title = title;
  s.message = message;
  DialogField f;
  f.initial = initial;
  f.select_from = select_from;
  f.select_to = select_to;
  s.fields.push_back(f);
  s.buttons = {tr("Cancel"), action};
  return ask(win, std::move(s), [done](const DialogAnswer &a) {
    if (done)
      done(a.button == 1, a.values.empty() ? std::string() : a.values[0]);
  });
}

Dialog *Dialog::ask(Window *win, DialogSpec spec, std::function<void(const DialogAnswer &)> done) {
  if (!win)
    return nullptr;
  auto *d = new Dialog();
  if (spec.buttons.empty())
    spec.buttons = {tr("Cancel"), tr("OK")};
  d->spec_ = std::move(spec);
  d->done_ = std::move(done);
  d->build(win);
  int ff = d->spec_.focus_field;
  if (!win->shown())
    return d; // tests: no display, no keyboard focus
  if (ff >= 0 && ff < int(d->fields_.size()))
    d->fields_[size_t(ff)]->take_focus();
  else
    d->buttons_.back()->take_focus();
  return d;
}

void Dialog::build(Window *win) {
  begin();
  for (const auto &f : spec_.fields) {
    auto *field = new TextField(0, 0, 100, kFieldH);
    field->placeholder(f.placeholder);
    field->secret(f.secret);
    field->set_text(f.initial, f.select_from, f.select_to);
    field->on_escape([this] { cancel(); });
    field->on_enter([this, field] {
      // Enter in a field moves to the next one, the last one answers.
      for (size_t i = 0; i + 1 < fields_.size(); ++i)
        if (fields_[i] == field) {
          fields_[i + 1]->take_focus();
          return;
        }
      accept();
    });
    field->on_change([this] {
      if (!error_.empty()) {
        error_.clear();
        relayout();
        redraw_all();
      }
    });
    fields_.push_back(field);
  }
  if (!spec_.check.empty()) {
    check_ = new CheckBox(0, 0, 100, 24, nullptr);
    check_->copy_label(spec_.check.c_str());
    check_->checked(spec_.checked);
  }
  for (size_t i = 0; i < spec_.buttons.size(); ++i) {
    auto *b = new Button(0, 0, 88, kButtonH, nullptr);
    b->copy_label(spec_.buttons[i].c_str());
    b->primary(i + 1 == spec_.buttons.size());
    int index = int(i);
    b->on_click([this, index] { choose(index); });
    buttons_.push_back(b);
  }
  end();
  shown_.jump(0);
  open(win);
  shown_.set(1, anim::ms::POPUP_IN, anim::Ease::EaseOut);
}

void Dialog::relayout() {
  // Measuring text needs the display: a window not shown yet lays the card
  // out when it first draws.
  if (!host() || !host()->shown()) {
    cw_ = 0;
    return;
  }
  cw_ = std::min(w() - 64, fields_.empty() && buttons_.size() <= 2 ? 480 : 520);
  int inner = cw_ - 2 * kPad;
  lines_ = spec_.message.empty()
               ? std::vector<std::string>()
               : text::wrap(spec_.message, inner, text::fltk_measure(text::REGULAR, text::size::BODY), 8);
  ch_ = 2 * kPad + kTitleH;
  if (!lines_.empty())
    ch_ += kGap + int(lines_.size()) * kLineH;
  ch_ += int(fields_.size()) * (kGap + kFieldH);
  if (!error_.empty())
    ch_ += 6 + kLineH;
  if (check_)
    ch_ += kGap + 24;
  ch_ += kGap + 4 + kButtonH;
  cx_ = (w() - cw_) / 2;
  cy_ = std::max(16, (h() - ch_) / 2);
  int yy = cy_ + kPad + kTitleH + (lines_.empty() ? 0 : kGap + int(lines_.size()) * kLineH);
  for (auto *f : fields_) {
    f->resize(cx_ + kPad, yy + kGap, inner, kFieldH);
    yy += kGap + kFieldH;
  }
  if (!error_.empty())
    yy += 6 + kLineH;
  if (check_) {
    check_->resize(cx_ + kPad, yy + kGap, std::min(inner, check_->preferred_width()), 24);
    yy += kGap + 24;
  }
  int by = yy + kGap + 4;
  int right = cx_ + cw_ - kPad;
  for (size_t i = buttons_.size(); i-- > 0;) {
    int bw = std::max(88, buttons_[i]->preferred_width());
    buttons_[i]->resize(right - bw, by, bw, kButtonH);
    right -= bw + 10;
  }
}

void Dialog::set_error(const std::string &text) {
  error_ = text;
  relayout();
  redraw_all();
}

void Dialog::finish(int button) {
  auto done = std::move(done_);
  DialogAnswer a;
  a.button = button;
  for (auto *f : fields_)
    a.values.push_back(f->value());
  a.checked = check_ && check_->checked();
  close();
  if (done)
    done(a);
}

void Dialog::choose(int index) {
  if (index < 0 || index >= int(buttons_.size())) {
    finish(-1);
    return;
  }
  if (index + 1 == int(buttons_.size()) && !fields_.empty() && validate_ && !validate_(fields_[0]->value()))
    return;
  finish(index);
}

void Dialog::accept() { choose(int(buttons_.size()) - 1); }

void Dialog::cancel() { finish(-1); }

int Dialog::handle(int event) {
  switch (event) {
  case FL_KEYBOARD:
  case FL_SHORTCUT:
    if (Fl::event_key() == FL_Escape) {
      cancel();
      return 1;
    }
    if (Fl::event_key() == FL_Enter || Fl::event_key() == FL_KP_Enter) {
      for (size_t i = 0; i < buttons_.size(); ++i)
        if (Fl::focus() == buttons_[i]) {
          choose(int(i));
          return 1;
        }
      if (Fl::focus() == check_ && event == FL_KEYBOARD)
        return Fl_Group::handle(event);
      accept();
      return 1;
    }
    if (event == FL_KEYBOARD)
      return Fl_Group::handle(event);
    return 1; // a decision: nothing else reaches the app meanwhile
  case FL_PUSH: {
    int ex = Fl::event_x(), ey = Fl::event_y();
    if (ex < cx_ || ex >= cx_ + cw_ || ey < cy_ || ey >= cy_ + ch_) {
      if (spec_.scrim_cancels)
        cancel();
      return 1;
    }
    Fl_Group::handle(event);
    return 1;
  }
  case FL_FOCUS:
    return 1;
  case FL_MOUSEWHEEL:
    return 1;
  }
  Fl_Group::handle(event);
  return event == FL_MOVE || event == FL_ENTER || event == FL_DRAG || event == FL_RELEASE ? 1 : 0;
}

void Dialog::draw() {
  const Theme &t = theme();
  if (cw_ == 0)
    relayout();
  double a = shown_.get(this);
  paint::fill(x(), y(), w(), h(), Rgb(0, 0, 0), kScrim * a);
  paint::push_opacity();
  draw_card(cx_, cy_, cw_, ch_, spec_.danger ? t.red() : t.accent());
  int inner = cw_ - 2 * kPad;
  text::draw(spec_.title, cx_ + kPad, cy_ + kPad, inner, kTitleH, t.bright_foreground(), text::SEMIBOLD,
             text::size::HEADING);
  int yy = cy_ + kPad + kTitleH + kGap;
  for (const auto &l : lines_) {
    text::draw(l, cx_ + kPad, yy, inner, kLineH, t.foreground());
    yy += kLineH;
  }
  if (!error_.empty() && !fields_.empty())
    text::draw(error_, cx_ + kPad, fields_.back()->y() + kFieldH + 6, inner, kLineH, t.red(), text::REGULAR,
               text::size::LABEL);
  draw_children();
  paint::pop_opacity(a);
}

// --- Toast ------------------------------------------------------------------

Toast::~Toast() {
  Fl::remove_timeout(expire, this);
}

Toast *Toast::show(Window *win, const std::string &text, double seconds, Kind kind) {
  if (!win)
    return nullptr;
  for (int i = win->children(); i-- > 0;)
    if (auto *old = dynamic_cast<Toast *>(win->child(i)))
      old->close();
  auto *t = new Toast();
  t->text_ = text;
  t->kind_ = kind;
  t->shown_.jump(0);
  t->open(win);
  t->shown_.set(1, anim::ms::POPUP_IN, anim::Ease::EaseOut);
  if (seconds > 0)
    Fl::add_timeout(seconds, expire, t);
  return t;
}

void Toast::expire(void *data) {
  auto *t = static_cast<Toast *>(data);
  if (t->leaving_)
    return;
  t->leaving_ = true;
  t->shown_.set(0, anim::ms::POPUP_OUT, anim::Ease::InCubic);
  if (!t->shown_.running())
    t->close();
  else
    anim::request_frame(step, t);
  t->redraw_all();
}

void Toast::step(void *data) {
  auto *t = static_cast<Toast *>(data);
  if (!t->is_open())
    return;
  t->shown_.value();
  if (t->shown_.running()) {
    anim::request_frame(step, t);
    t->redraw_all();
  } else {
    t->close();
  }
}

int Toast::handle(int event) {
  int ex = Fl::event_x(), ey = Fl::event_y();
  bool inside = ex >= cx_ && ex < cx_ + cw_ && ey >= cy_ && ey < cy_ + ch_;
  if (event == FL_PUSH && inside) {
    expire(this);
    return 1;
  }
  return 0;
}

void Toast::draw() {
  const Theme &t = theme();
  double a = shown_.get(this);
  int tw = int(text::width(text_, text::REGULAR, text::size::BODY)) + 40;
  cw_ = std::min(w() - 64, std::max(160, tw));
  ch_ = 40;
  cx_ = (w() - cw_) / 2;
  cy_ = h() - ch_ - 24 + int((1 - a) * 8);
  paint::push_opacity();
  paint::round_fill(cx_, cy_, cw_, ch_, t.radius(), t.lighter_background());
  paint::round_frame(cx_, cy_, cw_, ch_, t.radius(), 1, kind_ == Error ? t.red() : t.hairline());
  text::draw(text_, cx_ + 20, cy_, cw_ - 40, ch_, kind_ == Error ? t.red() : t.bright_foreground(), text::REGULAR,
             text::size::BODY, FL_ALIGN_CENTER);
  paint::pop_opacity(a);
}

// --- Tooltip ------------------------------------------------------------------

namespace {
std::unique_ptr<Fl_Widget_Tracker> g_tip;
}

void Tooltip::open_for(Fl_Widget *anchor, const std::string &text) {
  close_all();
  Window *win = Window::of(anchor);
  if (!win || text.empty())
    return;
  auto *t = new Tooltip();
  t->text_ = text;
  // Anchor in window coordinates (through any subwindows).
  int ax = anchor->x(), ay = anchor->y();
  for (Fl_Window *w = anchor->window(); w && w != win; w = w->window())
    ax += w->x(), ay += w->y();
  t->ax_ = ax, t->ay_ = ay, t->aw_ = anchor->w(), t->ah_ = anchor->h();
  t->open(win);
  g_tip.reset(new Fl_Widget_Tracker(t));
}

void Tooltip::close_all() {
  if (g_tip && !g_tip->deleted())
    static_cast<Tooltip *>(g_tip->widget())->close();
  g_tip.reset();
}

void Tooltip::draw() {
  const Theme &t = theme();
  int tw = int(text::width(text_, text::REGULAR, text::size::LABEL)) + 18;
  int th = 26;
  int tx = std::max(4, std::min(ax_ + (aw_ - tw) / 2, w() - tw - 4));
  int ty = ay_ + ah_ + 6;
  if (ty + th > h() - 4)
    ty = ay_ - th - 6;
  paint::round_fill(tx, ty, tw, th, t.radius_small(), t.lighter_background());
  paint::round_frame(tx, ty, tw, th, t.radius_small(), 1, t.hairline());
  text::draw(text_, tx + 9, ty, tw - 18, th, t.bright_foreground(), text::REGULAR, text::size::LABEL);
}

// --- HelpOverlay --------------------------------------------------------------

HelpOverlay *HelpOverlay::show(Window *win, std::vector<Row> rows, const std::string &title) {
  if (!win)
    return nullptr;
  auto *o = new HelpOverlay();
  o->rows_ = std::move(rows);
  o->title_ = title;
  o->shown_.jump(0);
  o->open(win);
  Fl::focus(o);
  o->shown_.set(1, anim::ms::POPUP_IN, anim::Ease::EaseOut);
  return o;
}

int HelpOverlay::handle(int event) {
  switch (event) {
  case FL_KEYBOARD:
  case FL_SHORTCUT:
    if (Fl::event_key() == FL_Escape || event_is("?") || Fl::event_key() == FL_Enter) {
      close();
      return 1;
    }
    return 1;
  case FL_PUSH:
    close();
    return 1;
  case FL_FOCUS:
  case FL_MOUSEWHEEL:
    return 1;
  }
  return 0;
}

void HelpOverlay::draw() {
  const Theme &t = theme();
  double a = shown_.get(this);
  paint::fill(x(), y(), w(), h(), Rgb(0, 0, 0), kScrim * a);
  const int row_h = 27, col_gap = 28, pair_gap = 48;
  int n = int(rows_.size());
  int avail_h = h() - 2 * kPad - kTitleH - kGap - 64;
  int per_col = std::max(1, avail_h / row_h);
  int kw = 0, dw = 0;
  for (const auto &r : rows_) {
    kw = std::max(kw, int(text::width(r.first, text::REGULAR, text::size::BODY)) + 1);
    dw = std::max(dw, int(text::width(r.second, text::REGULAR, text::size::BODY)) + 1);
  }
  int pairs = n > per_col ? 2 : 1;
  int max_w = w() - 64;
  int pair_w = kw + col_gap + dw;
  int cw = std::max(std::min(520, max_w), 2 * kPad + pairs * pair_w + (pairs - 1) * pair_gap);
  if (cw > max_w) {
    cw = max_w;
    dw = std::max(40, (cw - 2 * kPad - (pairs - 1) * pair_gap) / pairs - kw - col_gap);
    pair_w = kw + col_gap + dw;
  }
  int rows_per = pairs == 2 ? (n + 1) / 2 : n;
  int ch = 2 * kPad + kTitleH + 14 + rows_per * row_h;
  ch = std::min(ch, h() - 32);
  int cx = (w() - cw) / 2, cy = std::max(16, (h() - ch) / 2);
  paint::push_opacity();
  draw_card(cx, cy, cw, ch, t.accent());
  text::draw(title_.empty() ? tr("Keyboard shortcuts") : title_, cx + kPad, cy + kPad, cw - 2 * kPad, kTitleH,
             t.bright_foreground(), text::SEMIBOLD, text::size::HEADING);
  fl_push_clip(cx, cy, cw, ch - kPad / 2);
  for (int i = 0; i < n; ++i) {
    int col = pairs == 2 && i >= rows_per ? 1 : 0;
    int row = col ? i - rows_per : i;
    int rx = cx + kPad + col * (pair_w + pair_gap);
    int ry = cy + kPad + kTitleH + 14 + row * row_h;
    text::draw(rows_[size_t(i)].first, rx, ry, kw, row_h, t.accent(), text::REGULAR, text::size::BODY,
               FL_ALIGN_LEFT, text::Elide::None);
    text::draw(rows_[size_t(i)].second, rx + kw + col_gap, ry, dw, row_h, t.foreground());
  }
  fl_pop_clip();
  paint::pop_opacity(a);
}

} // namespace fo
