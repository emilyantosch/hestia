// Pure-QML stand-in for the Rust HestiaBackend (see crates/app/src/cxxqt_object.rs).
import QtQuick

QtObject {
    id: root

    property var libraryNames: ["Holiday photos", "Work documents"]
    property bool busy: false
    property bool ready: false
    property string error: ""
    property string status: ""

    signal operationFinished()

    // Simulates async backend latency.
    property Timer _timer: Timer {
        interval: 400
        onTriggered: {
            root.busy = false
            root.operationFinished()
        }
    }

    function _start(statusText) {
        root.error = ""
        root.busy = true
        root.status = statusText
        _timer.restart()
    }

    function refreshLibraries() {
        _start("")
    }

    function openLibrary(index) {
        root.ready = true
        _start(qsTr("Opened library \"%1\"").arg(libraryNames[index]))
    }

    function createLibrary(name, folder) {
        if (name.trim().length === 0) {
            root.error = qsTr("Library name must not be empty")
            return
        }
        libraryNames = libraryNames.concat([name])
        root.ready = true
        _start(qsTr("Created library \"%1\" at %2").arg(name).arg(folder))
    }

    function scan() {
        _start(qsTr("Scanning…"))
    }
}
