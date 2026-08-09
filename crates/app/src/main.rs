#[expect(
    clippy::unnecessary_box_returns,
    unsafe_code,
    unreachable_pub,
    reason = "CXX-Qt generates boxed public FFI exports and unsafe property accessors"
)]
mod cxxqt_object;

use controllers::AppController;
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use std::sync::Arc;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

fn init_tracing() {
    let filter = std::env::var("RUST_LOG").map_or_else(
        |_| EnvFilter::new("info"),
        |_| EnvFilter::from_default_env(),
    );
    let fmt_layer = fmt::layer()
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}

fn main() {
    init_tracing();

    let Ok(backend_runtime) = tokio::runtime::Runtime::new() else {
        tracing::error!("Could not start the backend task runtime.");
        return;
    };
    let Ok(controller) = AppController::new() else {
        tracing::error!("Could not initialize Hestia's application data directory.");
        return;
    };
    if let Err(error) =
        cxxqt_object::initialize(Arc::new(controller), backend_runtime.handle().clone())
    {
        tracing::error!(error = %error, "Could not initialize the Qt backend");
        return;
    }

    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/com/hestia/app/qml/Main.qml"));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
}
