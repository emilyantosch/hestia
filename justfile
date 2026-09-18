default: strict-lint test fmt

strict-lint:
    nix develop --command cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    nix develop --command cargo test --workspace

fmt:
    nix develop --command cargo fmt --all

# --- QML frontend without a cargo build -------------------------------------
# Runs qml/Main.qml against pure-QML mocks of the Rust types (crates/app/qml-mock).

# Open the UI in a window with mocked backend (edit .qml, re-run: ~1s turnaround)
qml-preview:
    nix develop --command sh -c 'cd crates/app && qml -I qml-mock qml/Main.qml'

# Live-reloading preview: save any .qml file and the window reloads in place.
# Note: reload re-instantiates the root, so UI state (selection, search) resets.
qml-watch:
    nix develop --command sh -c 'cd crates/app && qmlpreview qml -I qml-mock qml/Main.qml'

# Headless smoke test: fails if Main.qml has load/parse errors
# NB: this Qt build routes qDebug/qWarning to journald, so WITHOUT
# QT_FORCE_STDERR_LOGGING=1 nothing reaches stderr and this check passes
# vacuously — QML errors included.
qml-check:
    nix develop --command sh -c 'cd crates/app && QT_FORCE_STDERR_LOGGING=1 QT_QPA_PLATFORM=offscreen timeout 3 qml -I qml-mock qml/Main.qml 2>&1 | grep -iv "fontconfig\|placeholder.svg" | grep -qi "error\|qml:" && exit 1 || echo "Main.qml loaded OK"'

# Static lint of the QML (type resolution, unqualified access, etc.)
qml-lint:
    nix develop --command sh -c 'cd crates/app && qmllint -I qml-mock -I "$QML_IMPORT_PATH" qml/Main.qml qml-mock/com/hestia/app/*.qml'

qml-fmt:
    nix develop --command sh -c 'cd crates/app && qmlformat -i qml/Main.qml qml-mock/com/hestia/app/*.qml'
