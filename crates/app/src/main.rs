mod demo;
mod platform;
mod preview;
mod ui;

use anyhow::Result;
use gpui_kit::assets::AllAssets;
use gpui_kit::component::{Root, Theme};
use gpui_kit::{AppContext as _, WindowBounds, WindowOptions, px, size};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Child WebViews need GPUI's non-DirectComposition renderer on Windows.
    #[cfg(target_os = "windows")]
    #[expect(
        unsafe_code,
        reason = "Set renderer configuration before starting any threads"
    )]
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true");
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let runtime = tokio::runtime::Runtime::new()?;
    let library = runtime.block_on(demo::DemoLibrary::load())?;

    gpui_kit::application()
        .with_assets(AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            platform::init(cx);
            Theme::sync_system_appearance(None, cx);
            ui::configure_theme(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1320.), px(860.)), cx)),
                window_min_size: Some(size(px(1080.), px(680.))),
                ..Default::default()
            };
            cx.spawn(async move |cx| {
                let result = cx.open_window(options, |window, cx| {
                    window.set_window_title("Hestia — GPUI discovery");
                    let view = cx.new(|cx| ui::FileManager::new(library, window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                });
                match result {
                    Ok(window) => {
                        if let Err(error) =
                            window.update(cx, |_, window, _| window.activate_window())
                        {
                            tracing::error!(%error, "Could not activate Hestia");
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "Could not open Hestia");
                        cx.update(|cx| cx.quit());
                    }
                }
            })
            .detach();
            cx.activate(true);
        });
    Ok(())
}
