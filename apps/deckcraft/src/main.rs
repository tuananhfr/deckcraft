//! DeckCraft desktop app.
//!
//! Usage: `deckcraft [--control <port>] [--sample] [--show] [files…]`
//!
//! `--control <port>` (or `DECKCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server:
//! `{"id":1,"method":"ui.inspect","params":{}}` → `{"id":1,"ok":true,"result":…}`.
//! See `deckcraft_ui_egui::control` for the methods.
//!
//! `log` records go to standard error and `<settings dir>/logs/deckcraft.log` (`RUST_LOG` sets the
//! levels); see [`logging`].
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod audio;
mod control_server;
#[cfg(any(target_os = "windows", test))]
mod graphics;
mod logging;
mod settings;

use deckcraft_engine::Session;
use deckcraft_ui_egui::i18n;
use deckcraft_ui_egui::{Services, SlideApp};

struct App(SlideApp, String);

impl App {
    fn save_language_change(&mut self) {
        if self.0.ui.language != self.1
            && std::env::var_os("DECKCRAFT_NO_PREFS").is_none()
            && let Some(path) = prefs_path()
            && let Err(e) = settings::save_language_change(&path, &self.0.ui_state_to_save(), &mut self.1)
        {
            log::warn!("save UI language: {e}");
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.0.logic(ctx);
        self.save_language_change();
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.0.raw_input_hook(raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
        self.save_language_change();
    }
    fn on_exit(&mut self) {
        save_prefs(&self.0);
    }
}

/// The per-user settings directory: `ui.json`, `prefs.json` and `logs/` live here.
fn config_dir() -> Option<std::path::PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support/ERPCons/TrinhChieu"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("ERPCons/TrinhChieu"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
            .map(|c| c.join("ERPCons/TrinhChieu"))
    }
}

fn recovery_dir() -> Option<std::path::PathBuf> {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        config_dir().map(|d| d.join("Recovery"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
            .map(|d| d.join("ERPCons/TrinhChieu/recovery"))
    }
}

fn prefs_path() -> Option<std::path::PathBuf> {
    config_dir().map(|b| b.join("ui.json"))
}

fn load_prefs(app: &mut SlideApp) {
    app.ui.language = settings::language(None).into();
    if std::env::var_os("DECKCRAFT_NO_PREFS").is_some() {
        return;
    }
    let bytes = prefs_path().and_then(|p| std::fs::read(p).ok());
    app.ui = settings::load_ui(bytes.as_deref());
    if let Some(p) = prefs_path().map(|p| p.with_file_name("prefs.json"))
        && let Ok(bytes) = std::fs::read(&p)
        && let Ok(prefs) = serde_json::from_slice::<deckcraft_engine::Prefs>(&bytes)
    {
        app.session.prefs = prefs;
    }
}

fn save_prefs(app: &SlideApp) {
    if std::env::var_os("DECKCRAFT_NO_PREFS").is_some() {
        return;
    }
    if let Some(p) = prefs_path() {
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
        if let Err(e) = settings::save_ui(&p, &app.ui_state_to_save()) {
            log::warn!("save UI settings: {e}");
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&app.session.prefs) {
            let _ = std::fs::write(p.with_file_name("prefs.json"), bytes);
        }
    }
}

fn services() -> Services {
    let clipboard = std::rc::Rc::new(std::cell::RefCell::new(arboard::Clipboard::new().ok()));
    let copy_clipboard = clipboard.clone();
    let html_clipboard = clipboard.clone();
    Services {
        set_clipboard_html: Some(Box::new(move |html, text| {
            copy_clipboard.borrow_mut().as_mut().is_some_and(|cb| cb.set_html(html, Some(text)).is_ok())
        })),
        clipboard_html: Some(Box::new(move || html_clipboard.borrow_mut().as_mut()?.get().html().ok())),
        clipboard_text: Some(Box::new(move || clipboard.borrow_mut().as_mut()?.get_text().ok())),
        pick_open: Some(Box::new(|purpose: &str| {
            let d = rfd::FileDialog::new();
            let d = match purpose {
                "picture" => d.add_filter(i18n::t("Pictures"), &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"]),
                "audio" => d.add_filter(
                    i18n::t("Audio"),
                    &["wav", "mp3", "m4a", "m4b", "aac", "flac", "ogg", "oga", "opus", "aif", "aiff", "aifc", "caf", "wma", "weba", "mka"],
                ),
                "video" => d.add_filter(i18n::t("Video"), &["mp4", "m4v", "mov", "webm", "mkv", "wmv"]),
                _ => d
                    .add_filter(i18n::t("Presentations"), &["deckcraft", "pptx", "potx", "ppsx", "ppt", "pptm", "odp"])
                    .add_filter(PRODUCT_NAME, &["deckcraft"])
                    .add_filter("PowerPoint", &["pptx", "potx", "ppsx", "ppt", "pptm"])
                    .add_filter(i18n::t("Outline"), &["txt", "md"]),
            };
            d.pick_file().map(|p| p.to_string_lossy().to_string())
        })),
        pick_save: Some(Box::new(|name: &str| {
            let dialog = rfd::FileDialog::new().set_file_name(name);
            let dialog = match std::path::Path::new(name).extension().and_then(|ext| ext.to_str()) {
                Some("pptx") => dialog.add_filter("PowerPoint", &["pptx"]).add_filter(PRODUCT_NAME, &["deckcraft"]),
                Some("deckcraft") => dialog.add_filter(PRODUCT_NAME, &["deckcraft"]).add_filter("PowerPoint", &["pptx"]),
                _ => dialog,
            };
            dialog.save_file().map(|p| p.to_string_lossy().to_string())
        })),
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        clipboard_image: Some(Box::new(|| {
            let mut cb = arboard::Clipboard::new().ok()?;
            let img = cb.get_image().ok()?;
            let rgba = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())?;
            let mut out = Vec::new();
            rgba.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
            Some(out)
        })),
        // Media is mixed and clocked by the output device; `DECKCRAFT_NO_AUDIO` plays silently.
        audio_out: std::env::var_os("DECKCRAFT_NO_AUDIO")
            .is_none()
            .then(|| Box::new(audio::CpalOut::default()) as Box<dyn deckcraft_media::AudioOut>),
        ..Default::default()
    }
}

fn recovery_message(n: usize) -> String {
    let template = if n == 1 {
        i18n::t("Recovered {n} unsaved presentation from the last session")
    } else {
        i18n::t("Recovered {n} unsaved presentations from the last session")
    };
    i18n::fmt(template, &[("n", &n.to_string())])
}

const PRODUCT_NAME: &str = "Trình chiếu – ERPCons";
const APP_ID: &str = "vn.erpcons.trinhchieu";

fn brand_app(app: &mut SlideApp) {
    app.product_name = PRODUCT_NAME;
}

fn app_icon() -> Option<egui::IconData> {
    let png: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/vn.erpcons.trinhchieu.png");
    eframe::icon_data::from_png_bytes(png).map_err(|e| log::warn!("app icon: {e}")).ok()
}

fn main() -> eframe::Result {
    // First, so every start-up record (and the engine's panic hook, installed with the first
    // Session) is captured; see `logging`.
    let logger = logging::install();
    let mut control_port: Option<u16> = std::env::var("DECKCRAFT_CONTROL_PORT").ok().and_then(|p| p.parse().ok());
    let mut files = Vec::new();
    let mut sample = false;
    let mut show = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--control" => control_port = args.next().and_then(|p| p.parse().ok()),
            "--sample" => sample = true,
            "--show" => show = true,
            "--version" => {
                println!("deckcraft {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => files.push(a),
        }
    }
    log::info!("{PRODUCT_NAME} {} ({} {})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
    // The log file lives under the settings directory; opened after the arguments, so `--version`
    // leaves no file behind. Records logged until now are written to it first.
    if let Some(logger) = logger {
        match config_dir().map(|d| logger.attach_dir(&d.join(logging::LOG_DIR))) {
            Some(Ok(path)) => log::info!("log file {}", path.display()),
            Some(Err(e)) => log::warn!("no log file: {e}"),
            None => {
                logger.stderr_only();
                log::warn!("no log file: no settings directory (HOME, XDG_CONFIG_HOME or APPDATA is not set)");
            }
        }
    }
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(PRODUCT_NAME)
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        ..Default::default()
    };
    options.viewport = options.viewport.with_app_id(APP_ID);
    if let Some(icon) = app_icon() {
        options.viewport = options.viewport.with_icon(icon);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("DISPLAY").is_some() && std::env::var_os("DECKCRAFT_WAYLAND").is_none() {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    // Before eframe creates the wgpu instance: default Windows to DirectX 12 only (see graphics.rs).
    #[cfg(target_os = "windows")]
    graphics::configure(&mut options, eframe::wgpu::Backends::from_env());
    eframe::run_native(
        PRODUCT_NAME,
        options,
        Box::new(move |cc| {
            let mut app = SlideApp::new(Session::new(), services());
            brand_app(&mut app);
            app.product_icon = app_icon().map(|icon| {
                cc.egui_ctx.load_texture(
                    "erpcons-product",
                    egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba),
                    egui::TextureOptions::LINEAR,
                )
            });
            load_prefs(&mut app);
            i18n::set_current(i18n::Lang::from_pref(&app.ui.language));
            // AutoRecover: reopen what a previous run left unsaved, then keep it current.
            if std::env::var_os("DECKCRAFT_NO_RECOVERY").is_none() {
                app.session.recovery_dir = recovery_dir();
                match app.session.execute("file.recovery.open", &serde_json::json!({})) {
                    Ok(v) => {
                        if let Some(n) = v.get("opened").and_then(|o| o.as_array()).map(Vec::len).filter(|n| *n > 0) {
                            app.set_status(recovery_message(n));
                        }
                    }
                    Err(e) => {
                        log::warn!("recovery: {e}");
                        let message = i18n::t(
                            "Some unsaved presentations could not be recovered. Their recovery files remain on disk; close presentations and restart to try again.",
                        );
                        app.set_status(message);
                        rfd::MessageDialog::new()
                            .set_title(i18n::t("Presentation recovery"))
                            .set_description(message)
                            .set_level(rfd::MessageLevel::Warning)
                            .show();
                    }
                }
            }
            app.integrated_titlebar = cfg!(target_os = "macos");
            if let Some(port) = control_port {
                let rx = control_server::start(port, cc.egui_ctx.clone());
                app = app.with_control(rx);
            }
            if sample && let Err(e) = deckcraft_engine::sample::open_sample_with_product(&mut app.session, PRODUCT_NAME) {
                log::error!("sample: {e}");
            }
            for f in files {
                if let Err(e) = app.open_path(&f) {
                    log::error!("{f}: {e}");
                }
            }
            if show {
                app.start_show(0, false);
            }
            let language = app.ui.language.clone();
            Ok(Box::new(App(app, language)))
        }),
    )
}

#[cfg(test)]
mod desktop_tests {
    use super::*;

    #[test]
    fn desktop_branding_preserves_shared_default() {
        let mut app = SlideApp::new(Session::new(), Services::default());
        assert_eq!(app.product_name, "DeckCraft");
        brand_app(&mut app);
        assert_eq!(app.product_name, "Trình chiếu – ERPCons");
        assert_eq!(APP_ID, "vn.erpcons.trinhchieu");
    }

    #[test]
    fn desktop_sample_uses_host_product_name() {
        let mut session = Session::new();
        deckcraft_engine::sample::open_sample_with_product(&mut session, PRODUCT_NAME).unwrap();
        let doc = session.doc().unwrap().doc.as_ref();
        assert!(doc.slides.first().unwrap().notes_text().contains(PRODUCT_NAME));
        let data = serde_json::to_string(doc).unwrap();
        assert!(data.contains("Made with Trình chiếu – ERPCons — free and open source"));
        let mut shared = Session::new();
        deckcraft_engine::sample::open_sample(&mut shared).unwrap();
        assert!(shared.doc().unwrap().doc.slides.first().unwrap().notes_text().contains("made in DeckCraft"));
    }

    #[test]
    fn settings_and_recovery_use_erpcons_paths() {
        if let Some(dir) = config_dir() {
            assert!(dir.ends_with("ERPCons/TrinhChieu"), "{}", dir.display());
            assert_eq!(prefs_path(), Some(dir.join("ui.json")));
        }
        if let Some(dir) = recovery_dir() {
            let suffix =
                if cfg!(any(target_os = "macos", target_os = "windows")) { "ERPCons/TrinhChieu/Recovery" } else { "ERPCons/TrinhChieu/recovery" };
            assert!(dir.ends_with(suffix), "{}", dir.display());
        }
    }

    #[test]
    fn recovery_counts_are_translated() {
        i18n::set_current(i18n::Lang::EN);
        assert_eq!(recovery_message(1), "Recovered 1 unsaved presentation from the last session");
        assert_eq!(recovery_message(2), "Recovered 2 unsaved presentations from the last session");
        i18n::set_current(i18n::Lang::from_pref("vi"));
        assert_eq!(recovery_message(2), "Đã khôi phục 2 bản trình chiếu chưa lưu từ phiên trước");
        i18n::set_current(i18n::Lang::EN);
    }
}
