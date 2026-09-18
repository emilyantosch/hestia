pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import com.hestia.app

Pane {
    id: root
    required property HestiaBackend backend

    padding: Theme.spacing.lg

    background: Rectangle {
        color: Theme.color.surfaceVariant
        radius: Theme.radius.lg
        border.color: Theme.color.border
        border.width: Theme.stroke.hairline
    }

    FolderDialog {
        id: folderDialog
        title: qsTr("Choose the folder whose files Hestia should manage")
        onAccepted: root.backend.createLibrary(libraryName.text, selectedFolder)
    }

    contentItem: ColumnLayout {
        spacing: Theme.spacing.md

        Label {
            text: qsTr("Choose a Hestia library")
            color: Theme.color.text
            font.pixelSize: Theme.typography.heading.size
            font.weight: Theme.typography.heading.weight
        }

        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: Theme.spacing.xs
            model: root.backend.libraryNames
            delegate: Button {
                required property int index
                required property string modelData
                width: ListView.view.width
                height: Theme.size.control
                text: modelData
                font.pixelSize: Theme.typography.body.size
                onClicked: root.backend.openLibrary(index)
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing.sm

            TextField {
                id: libraryName
                Layout.fillWidth: true
                implicitHeight: Theme.size.control
                placeholderText: qsTr("New library name")
                enabled: !root.backend.busy
            }

            Button {
                text: qsTr("Choose folder and create")
                implicitHeight: Theme.size.control
                enabled: libraryName.text.trim().length > 0 && !root.backend.busy
                onClicked: folderDialog.open()
                palette.button: Theme.color.accent
                palette.buttonText: Theme.color.accentFg
            }
        }
    }
}
