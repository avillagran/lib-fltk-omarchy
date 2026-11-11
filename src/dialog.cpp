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
  auto *d = new Dialog();
  d->title_ = title;
  d->message_ = message;
  d->danger_ = danger;
  d->done_ = [done](bool ok, const std::string &) {
    if (done)
      done(ok);
  };
  d->build(win, false);
  d->ok_->copy_label(action.c_str());
  d->ok_->take_focus();
  return d;
}

Dialog *Dialog::prompt(Window *win, const std::string &title, const std::string &message, const std::string &initial,
                       const std::string &action, std::function<void(bool, const std::string &)> done,
                       int select_from, int select_to) {
  auto *d = new Dialog();
  d->title_ = title;
  d->message_ = message;
  d->done_ = std::move(done);
  d->build(win, true);
  d->ok_->copy_label(action.c_str());
  d->field_->set_text(initial, select_from, select_to);
  d->field_->take_focus();
  return d;
}

void Dialog::build(Window *win, bool with_field) {
  begin();
  if (with_field) {
    field_ = new TextField(0, 0, 100, kFieldH);
    field_->on_enter([this] { accept(); });
    field_->on_escape([this] { cancel(); });
    field_->on_change([this] {
      if (!error_.empty()) {
        error_.clear();
        relayout();
        redraw_all();
      }
    });
  }
  cancel_ = new Button(0, 0, 88, kButtonH, nullptr);
  cancel_->copy_label(tr("Cancel"));
  cancel_->on_click([this] { cancel(); });
  ok_ = new Button(0, 0, 88, kButtonH, nullptr);
  ok_->primary(true);
  ok_->on_click([this] { accept(); });
  end();
  shown_.jump(0);
  open(win);
  shown_.set(1, anim::ms::POPUP_IN, anim::Ease::EaseOut);
}

void Dialog::relayout() {
  if (!host())
    return;
  cw_ = std::min(w() - 64, field_ ? 520 : 480);
  int inner = cw_ - 2 * kPad;
  lines_ = message_.empty() ? std::vector<std::string>()
                            : text::wrap(message_, inner, text::fltk_measure(text::REGULAR, text::size::BODY), 8);
  ch_ = 2 * kPad + kTitleH;
  if (!lines_.empty())
    ch_ += kGap + int(lines_.size()) * kLineH;
  if (field_)
    ch_ += kGap + kFieldH;
  if (!error_.empty())
    ch_ += 6 + kLineH;
  ch_ += kGap + 4 + kButtonH;
  cx_ = (w() - cw_) / 2;
  cy_ = std::max(16, (h() - ch_) / 2);
  int yy = cy_ + kPad + kTitleH + (lines_.empty() ? 0 : kGap + int(lines_.size()) * kLineH);
  if (field_) {
    field_->resize(cx_ + kPad, yy + kGap, inner, kFieldH);
    yy += kGap + kFieldH;
  }
  if (!error_.empty())
    yy += 6 + kLineH;
  int bw = std::max(88, ok_->preferred_width());
  int cbw = std::max(88, cancel_->preferred_width());
  int by = yy + kGap + 4;
  ok_->resize(cx_ + cw_ - kPad - bw, by, bw, kButtonH);
  cancel_->resize(ok_->x() - 10 - cbw, by, cbw, kButtonH);
}

void Dialog::set_error(const std::string &text) {
  error_ = text;
  relayout();
  redraw_all();
}

void Dialog::finish(bool ok) {
  auto done = std::move(done_);
  std::string value = field_ ? field_->value() : "";
  close();
  if (done)
    done(ok, value);
}

void Dialog::accept() {
  if (field_ && validate_ && !validate_(field_->value()))
    return;
  finish(true);
}

void Dialog::cancel() { finish(false); }

int Dialog::handle(int event) {
  switch (event) {
  case FL_KEYBOARD:
  case FL_SHORTCUT:
    if (Fl::event_key() == FL_Escape) {
      cancel();
      return 1;
    }
    if (Fl::event_key() == FL_Enter || Fl::event_key() == FL_KP_Enter) {
      if (Fl::focus() == cancel_)
        cancel();
      else
        accept();
      return 1;
    }
    if (event == FL_KEYBOARD)
      return Fl_Group::handle(event);
    return 1; // a decision: nothing else reaches the app meanwhile
  case FL_PUSH: {
    int ex = Fl::event_x(), ey = Fl::event_y();
    if (ex < cx_ || ex >= cx_ + cw_ || ey < cy_ || ey >= cy_ + ch_) {
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
  double a = shown_.get(this);
  paint::fill(x(), y(), w(), h(), Rgb(0, 0, 0), kScrim * a);
  paint::push_opacity();
  draw_card(cx_, cy_, cw_, ch_, danger_ ? t.red() : t.accent());
  int inner = cw_ - 2 * kPad;
  text::draw(title_, cx_ + kPad, cy_ + kPad, inner, kTitleH, t.bright_foreground(), text::SEMIBOLD,
             text::size::HEADING);
  int yy = cy_ + kPad + kTitleH + kGap;
  for (const auto &l : lines_) {
    text::draw(l, cx_ + kPad, yy, inner, kLineH, t.foreground());
    yy += kLineH;
  }
  if (!error_.empty() && field_)
    text::draw(error_, cx_ + kPad, field_->y() + kFieldH + 6, inner, kLineH, t.red(), text::REGULAR,
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
