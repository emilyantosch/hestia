# QML mock module

Pure-QML stand-ins for the Rust-backed types in `src/cxxqt_object.rs`
(`HestiaBackend`, `FolderModel`, `FileModel`, `TagModel`) registered under
`com.hestia.app`. They let you iterate on `qml/Main.qml` with the `qml`
runtime instead of a full cargo/cxx-qt build.

```sh
just qml-preview   # open the window, mocked data, ~1s turnaround
just qml-check     # headless load test
just qml-lint      # qmllint
```

Keep the mock API in sync when you change `#[qproperty]` / `#[qinvokable]` /
`#[qsignal]` declarations or role names in `cxxqt_object.rs`. The real build
ignores this directory entirely (`build.rs` only picks up `qml/Main.qml`).
