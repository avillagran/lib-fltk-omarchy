//! Native Mermaid diagrams (feature `mermaid`): a flowchart, a sequence
//! diagram and a state diagram in `MermaidView`s, and a chat transcript
//! whose ```` ```mermaid ```` fences are drawn natively.
//!
//!   cargo run --features mermaid --example mermaid
//!   cargo run --features mermaid --example mermaid -- --snapshot build/mermaid   # PNGs, no window
//!   cargo run --release --features mermaid --example mermaid -- --bench         # timings and memory
use fltk::prelude::*;
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::mermaid::{self, MermaidView};
use fltk_omarchy::text::{Message, RichText};
use fltk_omarchy::{tr, App, AppWindow};
use std::time::Instant;

const FLOW: &str = "flowchart TD
    A[Request] --> B{Cached?}
    B -->|yes| C[Serve cache]
    B -->|no| D[Fetch origin]
    D --> E{Status}
    E -->|200| F[Store]
    E -->|404| G[Not found]
    E -->|5xx| H[Retry]
    H -.-> D
    F --> C
    C --> I((Done))
    G --> I
    subgraph auth [Auth]
        J[Token] --> K[Verify]
        K --> L{Valid?}
    end
    A --> J
    L -->|no| M[Reject]
    L -->|yes| B
    M --> I
    N[(Logs)] --- D
    O[[Metrics]] --- F
    P>Alert] -.-> H
    Q([Queue]) ==> D
    R{{Worker}} --> Q
    S[/Upload/] --> R
    T[\\Admin\\] --> S
";

const SEQ: &str = "sequenceDiagram
    autonumber
    participant U as User
    actor A as Admin
    participant S as Server
    participant D as Database
    U->>+S: Login
    S->>D: Query user
    D-->>S: Row
    S-->>-U: Token
    loop Every minute
        U->>S: Heartbeat
    end
    alt ok
        S->>D: Save
    else failure
        S-xU: Error
    end
    Note over S,D: Replicated
    A-)S: Audit
    S->>S: Log
";

const STATE: &str = "stateDiagram-v2
    direction LR
    [*] --> Idle
    Idle --> Running : start
    state Running {
        [*] --> Warmup
        Warmup --> Steady : ready
        Steady --> [*]
    }
    Running --> Idle : stop
    Running --> Failed : error
    state check <<choice>>
    Failed --> check
    check --> Idle : retry
    check --> [*] : give up
    note right of Failed : logs kept
";

const CHAT: &str = "Here is the deploy pipeline:

```mermaid
graph LR
  dev[Push] --> ci{CI green?}
  ci -->|yes| stage[Staging]
  ci -->|no| fix[Fix] --> dev
  stage --> prod[(Production)]
```

And how a request moves through it:

```mermaid
sequenceDiagram
  Client->>API: GET /deploys
  API->>DB: select
  DB-->>API: rows
  API-->>Client: 200 OK
```

The first block keeps a **Show source** action in its header.";

fn arg(name: &str) -> Option<String> {
    std::env::args().skip_while(|a| a != name).nth(1)
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn main() {
    if flag("--bench") {
        bench();
        return;
    }
    let app = App::new("dev.omarchy.fltk.mermaid");
    if let Some(dir) = arg("--snapshot") {
        fltk_omarchy::anim::set_enabled(false);
        std::fs::create_dir_all(&dir).expect("snapshot dir");
        for (name, src, w) in [("flowchart", FLOW, 760), ("sequence", SEQ, 760), ("state", STATE, 860)] {
            let (mut win, _view) = diagram_window(name, src, w);
            win.snapshot(format!("{dir}/mermaid-{name}.png")).expect("snapshot");
        }
        let (mut win, view) = chat_window();
        win.snapshot(format!("{dir}/mermaid-chat.png")).expect("snapshot");
        // A fence still streaming shows its code.
        view.set_message("a2", Message::markdown("Drawing it:\n\n```mermaid\ngraph TD\n  A --> B\n"));
        win.snapshot(format!("{dir}/mermaid-chat-streaming.png")).expect("snapshot");
        return;
    }
    let (mut win, _view) = chat_window();
    win.show();
    for (name, src, w) in [("Flowchart", FLOW, 760), ("Sequence", SEQ, 760), ("State", STATE, 860)] {
        let (mut w, _v) = diagram_window(name, src, w);
        w.show();
    }
    app.run();
}

fn diagram_window(title: &str, src: &str, w: i32) -> (AppWindow, MermaidView) {
    let probe = MermaidView::new(src);
    let inner = w - 24;
    let h = probe.preferred_height(inner);
    let mut win = AppWindow::new(title, w, h + 80);
    let mut body = win.body();
    body.set_margin(12);
    let mut view = MermaidView::new(src);
    view.set_size(inner, h);
    body.fit(&*view);
    win.end();
    (win, view)
}

fn chat_window() -> (AppWindow, RichText) {
    let mut win = AppWindow::new(&tr!("Chat"), 760, 900);
    let mut body = win.body();
    let view = RichText::new().native_mermaid(true);
    body.add(&*view);
    win.end();
    view.set_message("u1", Message::markdown("Can you sketch our deploy pipeline?"));
    view.set_message("a1", Message::markdown(CHAT));
    (win, view)
}

fn pss_kib() -> u64 {
    // Linux only (absent elsewhere: 0).
    std::fs::read_to_string("/proc/self/smaps_rollup")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("Pss:"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0)
}

fn scene_bytes(d: &mermaid::Diagram) -> usize {
    d.scene
        .items
        .iter()
        .map(|i| match i {
            mermaid::Item::Fill { points, .. } | mermaid::Item::Stroke { points, .. } => points.capacity() * 8 + 48,
            mermaid::Item::Text { lines, .. } => lines.iter().map(|l| l.capacity() + 24).sum::<usize>() + 64,
        })
        .sum()
}

/// Layout and rasterization time for a 20-node flowchart and a
/// 10-message sequence diagram, and the memory they take.
fn bench() {
    let app = App::new("dev.omarchy.fltk.mermaid-bench");
    fltk_omarchy::anim::set_enabled(false);
    let theme = fltk_omarchy::theme::current();
    let under = theme.palette.background;
    // Warm the font cache.
    let _ = mermaid::render(FLOW, 720.0);
    for (name, src) in [("flowchart (20 nodes, 23 edges, 1 subgraph)", FLOW), ("sequence (10 messages, 4 participants)", SEQ), ("state (composite, choice, note)", STATE)] {
        let n = 40;
        let t = Instant::now();
        let mut d = None;
        for _ in 0..n {
            d = Some(mermaid::render(src, 720.0).expect("render"));
        }
        let layout_ms = t.elapsed().as_secs_f64() * 1e3 / n as f64;
        let d = d.unwrap();
        let mut raster = Vec::new();
        for unit in [1.0f32, 2.0] {
            let (pw, ph) = ((d.width * unit).round() as usize, (d.height * unit).round() as usize);
            let t = Instant::now();
            for _ in 0..10 {
                let _ = mermaid::rasterize(&d, pw, ph, unit, &theme, under);
            }
            raster.push((unit, t.elapsed().as_secs_f64() * 1e3 / 10.0, pw * ph * 4));
        }
        println!(
            "{name}: {}x{} px, {} items; parse+layout {:.2} ms; scene {} KiB; shapes {:.2} ms @1x ({} KiB picture), {:.2} ms @2x ({} KiB)",
            d.width,
            d.height,
            d.scene.items.len(),
            layout_ms,
            scene_bytes(&d) / 1024,
            raster[0].1,
            raster[0].2 / 1024,
            raster[1].1,
            raster[1].2 / 1024
        );
    }
    let pss0 = pss_kib();
    let (mut win, view) = chat_window();
    win.show();
    for _ in 0..30 {
        fltk::app::wait_for(0.01).ok();
    }
    let pss1 = pss_kib();
    let mut wins = Vec::new();
    for (name, src, w) in [("f", FLOW, 760), ("s", SEQ, 760), ("st", STATE, 860)] {
        let (mut w, v) = diagram_window(name, src, w);
        w.show();
        wins.push((w, v));
    }
    for _ in 0..30 {
        fltk::app::wait_for(0.01).ok();
    }
    let pss2 = pss_kib();
    let t = Instant::now();
    for _ in 0..20 {
        for (w, _) in &mut wins {
            w.redraw();
        }
        fltk::app::flush();
        fltk::app::wait_for(0.0).ok();
    }
    let redraw_ms = t.elapsed().as_secs_f64() * 1e3 / 20.0 / 3.0;
    println!(
        "PSS: chat window with 2 native diagrams {} KiB (+{} KiB over the app), 3 MermaidView windows +{} KiB; cached redraw {:.2} ms per view",
        pss1,
        pss1.saturating_sub(pss0),
        pss2.saturating_sub(pss1),
        redraw_ms
    );
    let _ = (view, app);
}
