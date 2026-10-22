// kappa.yashi vector saver — native host. Free, provided "as is", no warranty, use at your own risk (see DISCLAIMER.md).
//
// Windows: build, rename kappayashi-saver.exe → kappayashi.scr, right-click → Install.
//   /s            run the saver (one borderless window per monitor)
//   /c  /c:<hwnd> settings window
//   /p <hwnd>     preview in the tiny Settings thumbnail — we just exit (blank thumbnail)
// Linux:  kappayashi-saver --saver     (start it from your idle hook: xss-lock, hypridle, swayidle …)
//         kappayashi-saver --config    (settings window)
//         add --windowed to test inside a normal window.
//
// The visuals live in ../../index.html (WebGL2). Any key / click / real mouse movement in the page tells
// the host to quit through window.ipc.postMessage('exit').

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{borrow::Cow, path::PathBuf};

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Fullscreen, Window, WindowBuilder},
};
use wry::{
    http::{header::CONTENT_TYPE, Response, StatusCode},
    WebContext, WebView, WebViewBuilder,
};

const SAVER_HTML: &str = include_str!("../../index.html");
const SETTINGS_HTML: &str = include_str!("../../settings.html");

// wry serves custom protocols as http://<name>.localhost on Windows, <name>://localhost elsewhere.
#[cfg(windows)]
const ORIGIN: &str = "http://kysaver.localhost";
#[cfg(not(windows))]
const ORIGIN: &str = "kysaver://localhost";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Saver,
    Preview,
    Config,
}

#[derive(Debug)]
enum UserEvent {
    Exit,
}

fn parse_args() -> (Mode, bool) {
    let mut mode = Mode::Config; // Windows runs a bare .scr for "configure"
    let mut windowed = false;
    for a in std::env::args().skip(1) {
        let a = a.to_ascii_lowercase();
        let t = a.trim_start_matches(|c| c == '/' || c == '-');
        if t == "windowed" {
            windowed = true;
        } else if t == "s" || t == "saver" {
            mode = Mode::Saver;
        } else if t == "p" || t.starts_with("p:") || t == "preview" {
            mode = Mode::Preview;
        } else if t == "c" || t.starts_with("c:") || t == "config" {
            mode = Mode::Config;
        }
    }
    (mode, windowed)
}

// Keep WebView2 / WebKit profile data (incl. localStorage = the saved settings) out of the .scr's own folder:
// a .scr usually lives in C:\Windows\System32, which we can't write to.
fn data_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("kappayashi-saver")
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

// Saved by the settings window, injected into every page as window.__KY_CFG (so the saver needs no web storage).
fn load_settings() -> String {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .filter(|v| v.is_object())
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".to_string())
}

fn save_settings(json: &str) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
        if v.is_object() {
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(settings_path(), v.to_string());
        }
    }
}

fn serve(path: &str) -> Response<Cow<'static, [u8]>> {
    let (status, ctype, body): (StatusCode, &str, &'static [u8]) = match path {
        "/" | "/index.html" => (StatusCode::OK, "text/html; charset=utf-8", SAVER_HTML.as_bytes()),
        "/settings.html" => (StatusCode::OK, "text/html; charset=utf-8", SETTINGS_HTML.as_bytes()),
        _ => (StatusCode::NOT_FOUND, "text/plain", &b"not found"[..]),
    };
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, ctype)
        .header("Cache-Control", "no-store")
        .body(Cow::Borrowed(body))
        .unwrap()
}

fn make_webview(window: &Window, ctx: &mut WebContext, url: String, proxy: EventLoopProxy<UserEvent>) -> WebView {
    let log = std::env::var_os("KY_LOG").is_some();
    let builder = WebViewBuilder::new_with_web_context(ctx)
        .with_background_color((0, 0, 0, 255))
        .with_custom_protocol("kysaver".into(), |_id, req| serve(req.uri().path()))
        .with_initialization_script(format!("window.__KY_CFG = {};", load_settings()))
        .with_url(url)
        .with_ipc_handler(move |req| {
            let m = req.body().as_str();
            if m == "exit" || m == "close" {
                let _ = proxy.send_event(UserEvent::Exit);
            } else if let Some(json) = m.strip_prefix("save ") {
                save_settings(json);
            } else if log {
                if let Some(rest) = m.strip_prefix("log ") {
                    eprintln!("[page] {rest}");
                }
            }
        });

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let webview = builder.build(window).expect("could not create the WebView (is the WebView2 runtime installed?)");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().expect("no GTK vbox");
        builder.build_gtk(vbox).expect("could not create the WebView (WebKitGTK missing?)")
    };
    webview
}

fn main() {
    let (mode, windowed) = parse_args();
    if mode == Mode::Preview {
        return; // nothing to show in the little preview window
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let mut ctx = WebContext::new(Some(data_dir()));
    let mut shown: Vec<(Window, WebView)> = Vec::new();

    match mode {
        Mode::Config => {
            let window = WindowBuilder::new()
                .with_title("kappa.yashi vector saver — settings")
                .with_inner_size(LogicalSize::new(640.0, 840.0))
                .build(&event_loop)
                .expect("window");
            let wv = make_webview(&window, &mut ctx, format!("{ORIGIN}/settings.html"), proxy.clone());
            shown.push((window, wv));
        }
        Mode::Saver => {
            let url = format!("{ORIGIN}/index.html?saver=1");
            if windowed {
                let window = WindowBuilder::new()
                    .with_title("kappa.yashi vector saver")
                    .with_inner_size(LogicalSize::new(1280.0, 720.0))
                    .build(&event_loop)
                    .expect("window");
                let wv = make_webview(&window, &mut ctx, url, proxy.clone());
                shown.push((window, wv));
            } else {
                let monitors: Vec<_> = event_loop.available_monitors().collect();
                let targets: Vec<Option<_>> = if monitors.is_empty() { vec![None] } else { monitors.into_iter().map(Some).collect() };
                for m in targets {
                    let window = WindowBuilder::new()
                        .with_title("kappa.yashi vector saver")
                        .with_decorations(false)
                        .with_always_on_top(true)
                        .with_fullscreen(Some(Fullscreen::Borderless(m)))
                        .build(&event_loop)
                        .expect("window");
                    window.set_cursor_visible(false);
                    let wv = make_webview(&window, &mut ctx, url.clone(), proxy.clone());
                    shown.push((window, wv));
                }
            }
        }
        Mode::Preview => unreachable!(),
    }

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let _keep_alive = (&shown, &ctx);
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(UserEvent::Exit) => {
                // exit right away: tearing WebView2 down gracefully can take a second
                std::process::exit(0);
            }
            _ => {}
        }
    });
}
