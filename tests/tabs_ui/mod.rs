//! Tab requests preserve app-owned pane/selection state and stable IDs.
use super::*;
mod plain_frames;
#[derive(Clone, PartialEq)]
struct Entry {
    key: u64,
    title: String,
}
fn model() -> ListModel<u64, Entry> {
    let model = ListModel::new(|t: &Entry| t.key);
    for key in [10, 20, 30] {
        model.push(Entry {
            key,
            title: "Same".into(),
        });
    }
    model
}
fn point(bar: &KeyedTabBar<u64>, key: u64, close: bool) -> (i32, i32) {
    if close {
        let (x, y, w, h) = bar.close_rect_for_key(&key).unwrap();
        return (x + w / 2, y + h / 2);
    }
    let (x, w) = bar.rect_for_key(&key).unwrap();
    (x + w / 2, bar.y() + bar.h() / 2)
}

pub fn checks(run: &mut dyn FnMut(&str, &dyn Fn(&Path) -> Result<(), String>)) {
    plain_frames::checks(run);
    run("tabs keyed: activating the same selected key reveals it after independent cursor scrolling", &|shots| {
        struct Restore;impl Drop for Restore{fn drop(&mut self){i18n::set_locale("en");}}
        let _restore=Restore;
        let model=ListModel::new(|t:&Entry|t.key);
        for key in 0..16 {model.push(Entry{key,title:format!("Tab {key:02}")});}
        let selected=State::new(Some(0));let changes=Rc::new(Cell::new(0));let c=changes.clone();
        selected.subscribe(move |_|c.set(c.get()+1)).forever();
        let mut win=AppWindow::new("Cursor scroll",320,160);let mut body=win.body();
        let bar=TabBar::from_model(&model,&selected,|t|t.title.clone());body.fit(&*bar);win.end();
        for locale in ["en","ar"]{
            i18n::set_locale(locale);
            win.snapshot(shots.join(format!("tabs-same-key-before-{locale}.png"))).map_err(|e|e.to_string())?;
            check(bar.focus_key(&15),"focus/reveal last without selection")?;
            win.snapshot(shots.join(format!("tabs-same-key-scrolled-{locale}.png"))).map_err(|e|e.to_string())?;
            check(selected.get()==Some(0)&&bar.rect_for_key(&0).is_none()&&bar.scroll_offset()>0,"independent cursor leaves first active key offscreen")?;
            focus::press("Home");
            check(bar.rect_for_key(&0).is_some()&&bar.scroll_offset()==0,"Home reveals same selected key immediately, without waiting for a draw")?;
            win.snapshot(shots.join(format!("tabs-same-key-home-{locale}.png"))).map_err(|e|e.to_string())?;
            check(selected.get()==Some(0)&&bar.focused_key()==Some(0)&&bar.rect_for_key(&0).is_some()&&changes.get()==0,"same-key activation restores viewport/cursor without owner selection notification")?;
        }
        fltk::window::DoubleWindow::delete((*win).clone());Ok(())
    });
    #[cfg(feature = "a11y")]
    run(
        "tabs keyed: accessible IDs survive rename/reorder and focus does not activate",
        &|shots| {
            use accesskit::Action;
            use accesskit::Role;
            use fltk_omarchy::a11y::testing;
            let model = model();
            for (key, name) in [(10, "A"), (20, "B"), (30, "C")] {
                model.patch(&key, |t| t.title = name.into());
            }
            let selected = State::new(Some(10));
            let mut win = AppWindow::new("Accessible tabs", 440, 160);
            let mut body = win.body();
            let bar = TabBar::from_model(&model, &selected, |t| t.title.clone());
            body.fit(&*bar);
            win.end();
            win.snapshot(shots.join("tabs-accessible.png"))
                .map_err(|e| e.to_string())?;
            let rec = testing::record(&*win);
            rec.sync();
            let id = rec.find(Role::Tab, "B").ok_or("B accessible node")?;
            rec.act(id, Action::Focus, None);
            check(
                selected.get() == Some(10) && bar.focused_key() == Some(20),
                "AT focus moves cursor without activation",
            )?;
            model.move_key(&20, 2);
            model.patch(&20, |t| t.title = "Renamed".into());
            rec.sync();
            check(
                rec.find(Role::Tab, "Renamed") == Some(id),
                "accessible child identity survives move/title patch",
            )?;
            rec.act(id, Action::Click, None);
            check(
                selected.get() == Some(20),
                "AT activation explicitly selects the keyed target",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "tabs keyed: composite pane/tab keys and actual close hit geometry under scroll/RTL",
        &|shots| {
            struct Restore;
            impl Drop for Restore {
                fn drop(&mut self) {
                    i18n::set_locale("en");
                }
            }
            let _restore = Restore;
            type Key = (u64, u64);
            let model = ListModel::new(|t: &(Key, String)| t.0);
            let keys = [(1, 7), (2, 7), (2, 9), (3, 8), (3, 9)];
            for key in keys {
                model.push((key, "Same".into()));
            }
            let selected = State::new(Some((1, 7)));
            let closed = Rc::new(RefCell::new(Vec::new()));
            let c = closed.clone();
            let contexts = Rc::new(RefCell::new(Vec::new()));
            let ctx = contexts.clone();
            let m = model.clone();
            let mut win = AppWindow::new("Composite keys", 360, 180);
            let mut body = win.body();
            let bar = TabBar::from_model(&model, &selected, |t| t.1.clone())
                .on_close(move |key| c.borrow_mut().push(key))
                .on_context(move |r| ctx.borrow_mut().push(r.key))
                .reorderable(move |r| {
                    r.apply_to(&m);
                });
            body.fit(&*bar);
            win.end();
            win.snapshot(shots.join("tabs-hit-ltr.png"))
                .map_err(|e| e.to_string())?;
            check(
                bar.overflowing() && bar.close_rect_for_key(&(3, 9)).is_none(),
                "hidden tab has no visible close hit area",
            )?;
            let arrow = (bar.x() + bar.w() - 2, bar.y() + bar.h() / 2);
            let offset = bar.scroll_offset();
            for button in [2, 3] {
                focus::click_button_at(&*win, arrow.0, arrow.1, button);
            }
            check(
                bar.hit_test(arrow.0, arrow.1).is_none()
                    && bar.scroll_offset() == offset
                    && closed.borrow().is_empty()
                    && contexts.borrow().is_empty(),
                "auxiliary arrow hits do not scroll, close or target a tab",
            )?;
            let (x, y, w, h) = bar.close_rect_for_key(&(2, 7)).ok_or("close rectangle")?;
            let p = (x + w / 2, y + h / 2);
            check(
                bar.hit_test(p.0, p.1)
                    == Some(TabHit {
                        key: (2, 7),
                        part: TabPart::Close,
                    }),
                "close rectangle matches actual hit test with compound key",
            )?;
            focus::right_click_at(&*win, p.0, p.1);
            focus::click_button_at(&*win, p.0, p.1, 2);
            check(
                contexts.borrow().as_slice() == [(2, 7)]
                    && closed.borrow().as_slice() == [(2, 7)]
                    && selected.get() == Some((1, 7)),
                "same tab_id in another pane never aliases active pane",
            )?;
            check(bar.focus_key(&(3, 9)), "reveal inactive offscreen cursor")?;
            focus::press("Mod+Shift+Left");
            check(
                model.key_at(3) == Some((3, 9)) && selected.get() == Some((1, 7)),
                "compound-key keyboard move preserves pane/tab selection",
            )?;
            for locale in ["en", "ar"] {
                i18n::set_locale(locale);
                win.snapshot(shots.join(format!("tabs-hit-{locale}.png")))
                    .map_err(|e| e.to_string())?;
                for key in keys {
                    if let Some((x, y, w, h)) = bar.close_rect_for_key(&key) {
                        check(
                            w > 0 && h > 0 && x >= bar.x() && x + w <= bar.x() + bar.w(),
                            "close rectangle clipped inside strip",
                        )?;
                        check(
                            bar.hit_test(x + w / 2, y + h / 2)
                                == Some(TabHit {
                                    key,
                                    part: TabPart::Close,
                                }),
                            "mirrored/scrolled close geometry agrees with hit test",
                        )?;
                    }
                }
            }
            check(
                bar.hit_test(bar.x() - 1, bar.y()).is_none()
                    && bar.hit_test(bar.x(), bar.y() - 1).is_none(),
                "outside points never return tab hits",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "tabs keyed: reentrant context, deferred model diffs, snapshot IDs and RTL cursor reorder",
        &|shots| {
            struct Restore;
            impl Drop for Restore {
                fn drop(&mut self) {
                    i18n::set_locale("en");
                }
            }
            let _restore = Restore;
            let model = model();
            let selected = State::new(Some(10));
            let changes = Rc::new(Cell::new(0));
            let c = changes.clone();
            selected.subscribe(move |_| c.set(c.get() + 1)).forever();
            let closed = Rc::new(RefCell::new(Vec::new()));
            let close = closed.clone();
            let ctx = Rc::new(RefCell::new(Vec::new()));
            let context = ctx.clone();
            let m = model.clone();
            let moves = model.clone();
            let mut win = AppWindow::new("Model updates", 540, 180);
            let mut body = win.body();
            let bar = TabBar::from_model(&model, &selected, |t| t.title.clone())
                .on_close(move |key| close.borrow_mut().push(key))
                .on_context(move |r| {
                    context.borrow_mut().push(r.key);
                    m.move_key(&r.key, 0);
                })
                .reorderable(move |r| {
                    r.apply_to(&moves);
                });
            body.fit(&*bar);
            win.end();
            win.snapshot(shots.join("tabs-snapshot-before.png"))
                .map_err(|e| e.to_string())?;
            let id = bar.id_for_key(&20);
            let width = bar.rect_for_key(&20).unwrap().1;
            let p = point(&bar, 20, true);
            focus::right_click_at(&*win, p.0, p.1);
            check(
                ctx.borrow().as_slice() == [20]
                    && closed.borrow().is_empty()
                    && selected.get() == Some(10),
                "context may move its tab reentrantly without selecting/closing",
            )?;
            let p = point(&bar, 20, false);
            model.transaction(|m| {
                m.move_key(&20, 2);
                focus::click_button_at(&*win, p.0, p.1, 2);
            });
            check(
                closed.borrow().as_slice() == [20],
                "pending model notification never remaps a painted tab index to another key",
            )?;
            model.sync_from(vec![
                Entry {
                    key: 30,
                    title: "C".into(),
                },
                Entry {
                    key: 20,
                    title: "A significantly longer renamed tab".into(),
                },
                Entry {
                    key: 10,
                    title: "A".into(),
                },
            ]);
            check(
                bar.id_for_key(&20) == id
                    && bar.rect_for_key(&20).unwrap().1 > width
                    && selected.get() == Some(10)
                    && changes.get() == 0,
                "snapshot rename/order retains identities and exact selection",
            )?;
            i18n::set_locale("ar");
            win.snapshot(shots.join("tabs-snapshot-rtl.png"))
                .map_err(|e| e.to_string())?;
            check(
                bar.rect_for_key(&30).unwrap().0 > bar.rect_for_key(&10).unwrap().0,
                "first keyed tab mirrors to the right",
            )?;
            check(bar.focus_key(&20), "focus RTL cursor")?;
            focus::press("Mod+Shift+Right");
            check(
                model.key_at(0) == Some(20) && selected.get() == Some(10) && changes.get() == 0,
                "RTL keyboard reorder uses logical direction without selecting",
            )?;
            model.sync_from(Vec::new());
            focus::press("Delete");
            bar.request_close(&20);
            check(
                selected.get() == Some(10) && changes.get() == 0 && closed.borrow().len() == 1,
                "empty model neither selects a replacement nor fires stale closes",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            model.push(Entry {
                key: 40,
                title: "After disposal".into(),
            });
            check(
                model.listener_count() == 0,
                "deleted tab strip removes its model listener on the next notification",
            )?;
            Ok(())
        },
    );
    run("tabs keyed: auxiliary buttons never select or fire primary close; context targets identity",&|shots|{
        let model=model();let selected=State::new(Some(10));
        let changes=Rc::new(Cell::new(0));let c=changes.clone();selected.subscribe(move |_|c.set(c.get()+1)).forever();
        let closes=Rc::new(RefCell::new(Vec::new()));let close=closes.clone();
        let contexts=Rc::new(RefCell::new(Vec::new()));let context=contexts.clone();
        let mut win=AppWindow::new("Buttons",540,180);let mut body=win.body();
        let bar=TabBar::from_model(&model,&selected,|t|t.title.clone()).on_close(move|key|close.borrow_mut().push(key)).on_context(move|r|context.borrow_mut().push(r));
        body.fit(&*bar);let field=TextField::new();body.fit(&*field);win.end();
        win.snapshot(shots.join("tabs-buttons.png")).map_err(|e|e.to_string())?;
        field.clone().take_focus().map_err(|e|e.to_string())?;
        let focused=focus::current().map(|w|w.as_widget_ptr());
        check(focused.is_some(),"pane input owns focus before auxiliary clicks")?;
        for close in [false,true]{
            let(x,y)=point(&bar,20,close);focus::click_button_at(&*win,x,y,3);
            check(selected.get()==Some(10)&&closes.borrow().is_empty(),"right body/cross only requests context")?;
            check(focus::current().map(|w|w.as_widget_ptr())==focused,"context does not steal focus from the pane")?;
        }
        check(contexts.borrow().iter().map(|r|r.key).collect::<Vec<_>>()==[20,20],"duplicate titles retain distinct context keys")?;
        for button in [8,9]{let(x,y)=point(&bar,20,true);focus::click_button_at(&*win,x,y,button);}
        check(closes.borrow().is_empty()&&contexts.borrow().len()==2&&selected.get()==Some(10),"side buttons have no primary/context effect")?;
        for close in [false,true]{let(x,y)=point(&bar,20,close);focus::click_button_at(&*win,x,y,2);}
        check(closes.borrow().as_slice()==[20,20]&&selected.get()==Some(10),"middle body/cross closes target without selecting")?;
        check(focus::current().map(|w|w.as_widget_ptr())==focused&&changes.get()==0,"auxiliary actions preserve exact active-pane focus/selection")?;
        let(x,y)=point(&bar,20,true);focus::click_at(&*win,x,y);
        check(closes.borrow().as_slice()==[20,20,20]&&selected.get()==Some(10),"left cross closes only its target")?;
        let(x,y)=point(&bar,30,false);focus::click_at(&*win,x,y);
        check(selected.get()==Some(30)&&changes.get()==1,"left body selects exactly once")?;
        fltk::window::DoubleWindow::delete((*win).clone());Ok(())
    });
    run(
        "tabs keyed: model patches/moves/removal preserve selection; gestures cancel stale IDs",
        &|shots| {
            let model = model();
            let selected = State::new(Some(10));
            let changes = Rc::new(Cell::new(0));
            let c = changes.clone();
            selected.subscribe(move |_| c.set(c.get() + 1)).forever();
            let closed = Rc::new(RefCell::new(Vec::new()));
            let out = closed.clone();
            let mut win = AppWindow::new("IDs", 540, 180);
            let mut body = win.body();
            let bar = TabBar::from_model(&model, &selected, |t| t.title.clone())
                .on_close(move |key| out.borrow_mut().push(key));
            body.fit(&*bar);
            win.end();
            win.snapshot(shots.join("tabs-ids.png"))
                .map_err(|e| e.to_string())?;
            let id = bar.id_for_key(&20);
            model.patch(&20, |t| t.title = "Renamed".into());
            model.move_key(&20, 0);
            check(
                bar.id_for_key(&20) == id && selected.get() == Some(10) && changes.get() == 0,
                "title/order changes preserve IDs and external selection",
            )?;
            let p = point(&bar, 20, true);
            focus::mouse_drag(&*win, p, p, false);
            model.move_key(&20, 2);
            let p = point(&bar, 20, true);
            focus::mouse_up(&*win, p.0, p.1);
            check(
                closed.borrow().as_slice() == [20],
                "close press follows captured identity after a model move",
            )?;
            let p = point(&bar, 20, true);
            focus::mouse_drag(&*win, p, p, false);
            model.remove_key(&20);
            model.push(Entry {
                key: 20,
                title: "Replacement".into(),
            });
            check(
                bar.id_for_key(&20) != id,
                "reinserted domain key gets a fresh gesture identity",
            )?;
            let p = point(&bar, 20, true);
            focus::mouse_up(&*win, p.0, p.1);
            check(
                closed.borrow().as_slice() == [20],
                "removed press ID cannot close its replacement",
            )?;
            model.remove_key(&10);
            check(
                selected.get() == Some(10) && changes.get() == 0,
                "removing selected key never implicitly changes pane selection",
            )?;
            bar.request_close(&999);
            check(closed.borrow().len() == 1, "invalid close key ignored")?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "tabs keyed: drag and keyboard reorder cursor without selecting; stable insertion requests",
        &|shots| {
            let model = model();
            let selected = State::new(Some(10));
            let changes = Rc::new(Cell::new(0));
            let c = changes.clone();
            selected.subscribe(move |_| c.set(c.get() + 1)).forever();
            let requests = Rc::new(RefCell::new(Vec::new()));
            let out = requests.clone();
            let m = model.clone();
            let contexts = Rc::new(RefCell::new(Vec::new()));
            let context = contexts.clone();
            let mut win = AppWindow::new("Reorder", 540, 180);
            let mut body = win.body();
            let bar = TabBar::from_model(&model, &selected, |t| t.title.clone())
                .reorderable(move |r| {
                    out.borrow_mut().push(r.clone());
                    r.apply_to(&m);
                })
                .on_context(move |r| context.borrow_mut().push(r.key));
            body.fit(&*bar);
            win.end();
            win.snapshot(shots.join("tabs-reorder.png"))
                .map_err(|e| e.to_string())?;
            let from = point(&bar, 20, false);
            let (x, w) = bar.rect_for_key(&30).unwrap();
            focus::mouse_drag(&*win, from, (x + w - 2, from.1), false);
            focus::press("Escape");
            focus::mouse_up(&*win, x + w - 2, from.1);
            check(
                requests.borrow().is_empty() && selected.get() == Some(10),
                "Escape cancels a drag without activation or reorder",
            )?;
            focus::mouse_drag(&*win, from, (x + w - 2, bar.y() + bar.h() + 10), true);
            check(
                requests.borrow().is_empty() && selected.get() == Some(10),
                "drop outside strip cancels without activation",
            )?;
            focus::mouse_drag(&*win, from, (x + w - 2, from.1), true);
            check(
                model.key_at(2) == Some(20) && selected.get() == Some(10) && changes.get() == 0,
                "dragging inactive tab reorders without activation",
            )?;
            check(
                requests.borrow().as_slice()
                    == [TabReorder {
                        key: 20,
                        before: None,
                    }],
                "drag returns stable key/end request",
            )?;
            check(bar.focus_key(&20), "focus inactive tab")?;
            focus::press("Ctrl+Shift+PageDown");
            focus::press("Ctrl+Shift+PageUp");
            check(
                selected.get() == Some(10) && requests.borrow().len() == 1,
                "app-owned move shortcuts never fall through to tab selection",
            )?;
            focus::press("Mod+Shift+Left");
            check(
                model.key_at(1) == Some(20) && selected.get() == Some(10) && changes.get() == 0,
                "keyboard moves focused inactive tab without activation",
            )?;
            check(
                requests.borrow().last()
                    == Some(&TabReorder {
                        key: 20,
                        before: Some(30),
                    }),
                "keyboard insertion is keyed",
            )?;
            focus::press("Menu");
            check(
                contexts.borrow().as_slice() == [20] && selected.get() == Some(10),
                "keyboard context targets cursor without activation",
            )?;
            focus::press("Alt+Right");
            check(
                bar.focused_key() == Some(30) && selected.get() == Some(10),
                "cursor navigation remains separate from selection",
            )?;
            focus::press("Enter");
            check(
                selected.get() == Some(30) && changes.get() == 1,
                "Enter explicitly activates cursor",
            )?;
            let before = model.key_at(0);
            TabReorder {
                key: 20,
                before: Some(999),
            }
            .apply_to(&model);
            check(
                model.key_at(0) == before,
                "missing insertion target cannot silently append",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
}
