import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.hestia.app

ApplicationWindow {
    id: window
    width: 1280
    height: 720
    visible: true
    title: qsTr("Hestia")

    // The page surface itself — the bottom of the surface stack. Panels
    // (surfaceVariant) and popovers (surfaceRaised) are drawn on top of it.
    color: Theme.color.surface

    // Theme hand-off to every Control below. `palette` is inherited down the
    // item tree, so Buttons, TextFields, ItemDelegates, ScrollBars and ToolTips
    // pick these up without each one overriding its own `background:`.
    // Map Hestia's semantic tokens onto Qt's palette roles:
    palette.window: Theme.color.surface
    palette.windowText: Theme.color.text
    // `base` is the fill of editable widgets (TextField): raised, so an input
    // reads as sitting on top of the surfaceVariant panel, not cut into it.
    palette.base: Theme.color.surfaceRaised
    palette.alternateBase: Theme.color.surfaceSunken
    palette.text: Theme.color.text
    // Buttons sit on surfaceVariant panels, so they take the sunken fill —
    // surfaceVariant would make them disappear into the panel behind them.
    palette.button: Theme.color.surfaceSunken
    palette.buttonText: Theme.color.text
    palette.mid: Theme.color.border
    palette.dark: Theme.color.borderStrong
    // Built-in selection reuses the same pair the delegates draw by hand.
    palette.highlight: Theme.color.selection
    palette.highlightedText: Theme.color.selectionFg
    palette.accent: Theme.color.accent
    palette.link: Theme.color.accentText
    palette.placeholderText: Theme.color.textFaint
    palette.toolTipBase: Theme.color.surfaceRaised
    palette.toolTipText: Theme.color.text

    HestiaBackend {
        id: backend
        Component.onCompleted: refreshLibraries()
    }

    // The app background: a real item (not just the window clear colour) so it
    // participates in the scene — grabbable, animatable on theme change, and
    // the bottom of the surface stack that panels are drawn on top of.
    Rectangle {
        id: appBackground
        anchors.fill: parent
        color: Theme.color.surface

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: Theme.spacing.lg
            spacing: Theme.spacing.md

            Label {
                Layout.fillWidth: true
                visible: backend.error.length > 0
                color: Theme.color.danger
                text: backend.error
                wrapMode: Text.Wrap
                font.pixelSize: Theme.typography.body.size
            }

            BusyIndicator {
                Layout.alignment: Qt.AlignHCenter
                running: backend.busy
                visible: running
            }

            WelcomeView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: !backend.ready
                backend: backend
            }

            LibraryView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: backend.ready
                backend: backend
            }
        }
    }
}
