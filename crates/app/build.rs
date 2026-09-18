use cxx_qt_build::{CxxQtBuilder, QResource, QResources, QmlFile, QmlModule};
use qt_build_utils::QResourceFile;

fn main() {
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.hestia.app")
            .qml_file("qml/Main.qml")
            .qml_file("qml/WelcomeView.qml")
            .qml_file("qml/LibraryView.qml")
            .qml_file(QmlFile::from("qml/Theme.qml").singleton(true)),
    )
        // Embed the logo next to the QML in the module's resource tree.
        // The alias strips the `qml/` source prefix so that `assets/...` is
        // relative to the module root in the built binary, exactly as it is
        // relative to qml/ in the qml-mock preview — one source string works
        // in both. The prefix (/qt/qml/com/hestia/app) is added automatically.
        .qrc_resources(QResources::new().resource(
            QResource::new().file(
                QResourceFile::new("qml/assets/hestia_logo.svg")
                    .alias("assets/hestia_logo.svg"),
            ),
        ))
        .qt_module("Network")
        .files(["src/cxxqt_object.rs"])
        .build();
}
