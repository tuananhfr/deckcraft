//! Renders the DeckCraft window offscreen to a PNG, with the sample deck open.
//!
//! ```sh
//! cargo run -p deckcraft-ui-egui --example snapshot -- [--size 390x844] [--language vi] [--pane format]
//!   [--tab Insert] [--expand-ribbon] [--thumbs] [--select 1] [--docs 3] [--cmd 'id={json}']... out.png
//! ```
//!
//! `--select n` selects the slide's n-th shape; `--thumbs` opens the slide thumbnails (they hide
//! in a narrow window); `--expand-ribbon` unfolds the ribbon a narrow window folds.

use deckcraft_engine::Session;
use deckcraft_ui_egui::{Services, SlideApp};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut size = (1366.0f32, 768.0f32);
    let mut language = "vi".to_string();
    let mut pane = None;
    let mut tab = None;
    let mut expand = false;
    let mut thumbs = false;
    let mut select = None;
    let mut docs = 1;
    let mut cmds = vec![];
    let mut out = "snapshot.png".to_string();
    let mut i = 0;
    while i < args.len() {
        let next = |i: &mut usize| {
            *i += 1;
            args.get(*i).cloned().unwrap_or_default()
        };
        match args[i].as_str() {
            "--size" => {
                let v = next(&mut i);
                let (w, h) = v.split_once('x').ok_or("--size WxH")?;
                size = (w.parse()?, h.parse()?);
            }
            "--language" => language = next(&mut i),
            "--pane" => pane = Some(next(&mut i)),
            "--tab" => tab = Some(next(&mut i)),
            "--expand-ribbon" => expand = true,
            "--thumbs" => thumbs = true,
            "--select" => select = Some(next(&mut i).parse::<usize>()?),
            "--docs" => docs = next(&mut i).parse()?,
            "--cmd" => cmds.push(next(&mut i)),
            other => out = other.to_string(),
        }
        i += 1;
    }

    let mut session = Session::new();
    for _ in 0..docs.max(1) {
        deckcraft_engine::sample::open_sample(&mut session)?;
    }
    let mut app = SlideApp::new(session, Services::default());
    app.ui.language = language;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(size.0, size.1)).wgpu().build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    // Fonts install on the first frames; the narrow layout settles on the next.
    harness.run_steps(3);
    let app = harness.state_mut();
    if let Some(t) = tab {
        app.run("view.tab", serde_json::json!({"tab": t}))?;
    }
    if expand {
        app.ui.ribbon_collapsed = false;
    }
    if thumbs {
        app.run("view.thumbnails", serde_json::json!({"on": true}))?;
    }
    if let Some(n) = select {
        let id = app.session.active().and_then(|d| d.current_slide().and_then(|s| s.shapes.get(n)).map(|s| s.id.0)).ok_or("no such shape")?;
        app.run("edit.select", serde_json::json!({"ids": [id]}))?;
    }
    if let Some(p) = pane {
        app.run("view.pane", serde_json::json!({"pane": p}))?;
    }
    for c in &cmds {
        let (id, p) = c.split_once('=').unwrap_or((c, "{}"));
        app.run(id, serde_json::from_str(p)?)?;
    }
    harness.run_steps(4);
    let image = harness.render()?;
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir)?;
    }
    image.save(&out)?;
    println!("wrote {out} ({}x{})", image.width(), image.height());
    Ok(())
}
