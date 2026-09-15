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

# Headless smoke test: fails if Main.qml has load/parse errors
qml-check:
    nix develop --command sh -c 'cd crates/app && QT_QPA_PLATFORM=offscreen timeout 3 qml -I qml-mock qml/Main.qml 2>&1 | grep -iv fontconfig | grep -qi "error\|qml:" && exit 1 || echo "Main.qml loaded OK"'

# Static lint of the QML (type resolution, unqualified access, etc.)
qml-lint:
    nix develop --command sh -c 'cd crates/app && qmllint -I qml-mock -I "$QML_IMPORT_PATH" qml/Main.qml qml-mock/com/hestia/app/*.qml'

qml-fmt:
    nix develop --command sh -c 'cd crates/app && qmlformat -i qml/Main.qml qml-mock/com/hestia/app/*.qml'
