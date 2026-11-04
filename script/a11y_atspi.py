"""Reads examples/a11y_check through AT-SPI (the bus Orca uses) and drives
it with AT actions only: no screen reader, no synthetic input.
    python3 -I script/a11y_atspi.py <app.log>
Prints the tree and one PASS/FAIL line per check."""
import sys, time
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

LOG = sys.argv[1]
APP = "a11y_check"
results = []

def check(ok, what):
    results.append(ok)
    print(("PASS " if ok else "FAIL ") + what, flush=True)

def log_has(text, timeout=3.0):
    end = time.time() + timeout
    while time.time() < end:
        with open(LOG) as f:
            if text in f.read():
                return True
        time.sleep(0.05)
    return False

def find_app(timeout=20.0):
    end = time.time() + timeout
    while time.time() < end:
        d = Atspi.get_desktop(0)
        for i in range(d.get_child_count()):
            a = d.get_child_at_index(i)
            if a is not None and a.get_name() == APP and a.get_child_count() > 0:
                return a
        time.sleep(0.2)
    return None

STATES = [("focused", Atspi.StateType.FOCUSED), ("checked", Atspi.StateType.CHECKED),
          ("selected", Atspi.StateType.SELECTED), ("expanded", Atspi.StateType.EXPANDED),
          ("collapsed", Atspi.StateType.COLLAPSED), ("pressed", Atspi.StateType.PRESSED),
          ("checkable", Atspi.StateType.CHECKABLE), ("focusable", Atspi.StateType.FOCUSABLE),
          ("editable", Atspi.StateType.EDITABLE)]

def text_of(o):
    t = o.get_text_iface()
    return Atspi.Text.get_text(t, 0, Atspi.Text.get_character_count(t))

# libatspi names: "button" (older releases: "push button").
BUTTON = {"button", "push button"}

def describe(o):
    s = o.get_state_set()
    states = [n for n, t in STATES if s.contains(t)]
    extra = ""
    try:
        v = o.get_value_iface()
        if v is not None:
            extra += " value=%g [%g..%g]" % (v.get_current_value(), v.get_minimum_value(), v.get_maximum_value())
    except Exception:
        pass
    try:
        if o.get_text_iface() is not None:
            extra += " text=%r" % text_of(o)
    except Exception:
        pass
    try:
        a = o.get_action_iface()
        if a is not None and a.get_n_actions() > 0:
            extra += " actions=" + ",".join(Atspi.Action.get_action_name(a, i) for i in range(a.get_n_actions()))
    except Exception:
        pass
    return "%s %r%s %s" % (o.get_role_name(), o.get_name(), extra, states)

def walk(o, depth, out, limit=200):
    if len(out) >= limit:
        return
    out.append((depth, o))
    for i in range(o.get_child_count()):
        c = o.get_child_at_index(i)
        if c is not None:
            walk(c, depth + 1, out, limit)

def find(root, role, name):
    out = []
    walk(root, 0, out)
    for _, o in out:
        r = o.get_role_name()
        if (r == role or (role in BUTTON and r in BUTTON)) and o.get_name() == name:
            return o
    return None

app = find_app()
check(app is not None, "the app is on the AT-SPI desktop (activated lazily when AT-SPI IsEnabled turns on)")
if app is None:
    sys.exit(1)
nodes = []
walk(app, 0, nodes)
print("--- tree (AT-SPI roles and names)")
for depth, o in nodes:
    print("  " * depth + describe(o))
print("---")
roles = {(o.get_role_name(), o.get_name()) for _, o in nodes}
roles |= {("push button", n) for r, n in roles if r in BUTTON}
for role, name in [("frame", "a11y check"), ("push button", "Press me"), ("push button", "Delete"), ("push button", "Preferences"),
                   ("entry", "Name"), ("toggle button", "Wi-Fi"), ("slider", "Volume"), ("combo box", "Output"),
                   ("label", "Pressed 0 times"), ("heading", "a11y check")]:
    check((role, name) in roles, "%s %r exposed" % (role, name))
items = [o for _, o in nodes if o.get_role_name() == "list item"]
check(0 < len(items) <= 30, "virtual list exposes %d rows of 10000" % len(items))

button = find(app, "push button", "Press me")
if button is not None:
    act = button.get_action_iface()
    check(act.do_action(0), "AT-SPI DoAction(%r) on 'Press me'" % Atspi.Action.get_action_name(act, 0))
    check(log_has("event pressed 1"), "the AT click ran the button's on_click (app log: 'event pressed 1')")
    time.sleep(0.3)
    check(find(app, "label", "Pressed 1 times") is not None, "the label update reached the AT-SPI tree incrementally")
switch = find(app, "toggle button", "Wi-Fi")
if switch is not None:
    switch.get_action_iface().do_action(0)
    check(log_has("event wifi false"), "AT-SPI DoAction on the switch toggled its State")
slider = find(app, "slider", "Volume")
if slider is not None:
    slider.get_value_iface().set_current_value(0.8)
    check(log_has("event volume 0.80"), "AT-SPI Value.SetCurrentValue(0.8) on the slider")
entry = find(app, "entry", "Name")
if entry is not None:
    check(text_of(entry) == "Ada", "AT-SPI Text on the field reads 'Ada'")
    ed = entry.get_editable_text_iface()
    if ed is not None:
        Atspi.EditableText.set_text_contents(ed, "Grace")
        time.sleep(0.3)
        check(text_of(entry) == "Grace", "AT-SPI EditableText.SetTextContents reached the field (SetValue)")
# Keyboard focus is reported by stable node: focus the list from the AT.
lst = next((o for _, o in nodes if o.get_role_name() == "list box"), None)
if lst is not None:
    first = lst.get_child_at_index(0)
    Atspi.Component.grab_focus(first.get_component_iface())
    time.sleep(0.4)
    check(first.get_state_set().contains(Atspi.StateType.FOCUSED) and first.get_state_set().contains(Atspi.StateType.SELECTED),
          "AT-SPI GrabFocus on a list row: focused and selected (Focus action -> list selection)")
print("RESULT %s (%d/%d)" % ("PASS" if all(results) else "FAIL", sum(results), len(results)), flush=True)
