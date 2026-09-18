pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtQuick.Layouts
import com.hestia.app

ColumnLayout {
    id: root
    required property HestiaBackend backend

    property int selectedFolderId: -1
    property int selectedFileId: -1

    spacing: Theme.spacing.md

    // Panels all share the same recipe: a surfaceVariant card on the page
    // surface, separated by a hairline border rather than by contrast alone.
    component PanelBackground: Rectangle {
        color: Theme.color.surfaceVariant
        radius: Theme.radius.lg
        border.color: Theme.color.border
        border.width: Theme.stroke.hairline
    }

    FolderModel {
        id: folders
    }
    FileModel {
        id: files
    }
    TagModel {
        id: tags
    }

    Component.onCompleted: {
        folders.refresh();
        files.refresh(root.selectedFolderId, search.text);
        tags.refresh();
    }

    Connections {
        target: root.backend
        function onOperationFinished() {
            if (root.backend.ready) {
                folders.refresh();
                files.refresh(root.selectedFolderId, search.text);
                tags.refresh();
            }
        }
    }

    // Top bar: same panel recipe as the SplitView panes, spanning full width.
    // Height comes from the content, so the bar stays as tall as one control
    // plus its padding rather than stretching with the window.
    //
    // Layout is left cluster + centred omnibox: the search pill is anchored to
    // the bar's centre (not laid out in a row), so it stays put no matter how
    // wide the status text next to it runs.
    Pane {
        Layout.fillWidth: true
        padding: Theme.spacing.md
        background: PanelBackground {}

        contentItem: Item {
            implicitHeight: Theme.size.control

            RowLayout {
                id: leftCluster
                anchors.left: parent.left
                anchors.right: omnibox.left
                anchors.rightMargin: Theme.spacing.md
                anchors.verticalCenter: parent.verticalCenter
                spacing: Theme.spacing.sm

                Image {
                    // Relative source: resolves to qml/assets/ in the qml-mock
                    // preview and to qrc:/qt/qml/com/hestia/app/assets/ in the
                    // built module, because build.rs aliases it to the same path.
                    source: "assets/hestia_transparent.svg"
                    fillMode: Image.PreserveAspectFit
                    Layout.preferredHeight: Theme.size.icon
                    // SVGs rasterise at sourceSize, so pin it to the drawn height
                    // or the logo renders blurry on high-DPI screens.
                    sourceSize.height: Theme.size.icon * Screen.devicePixelRatio
                    Layout.preferredWidth: implicitWidth * (Layout.preferredHeight / Math.max(implicitHeight, 1))
                    Layout.rightMargin: Theme.spacing.xs
                }

                Label {
                    Layout.fillWidth: true
                    text: root.backend.status
                    color: Theme.color.textMuted
                    font.pixelSize: Theme.typography.body.size
                    elide: Text.ElideRight
                }
            }

            // Omnibox: the field and its Scan action share one pill, the way a
            // browser keeps reload inside the URL bar. The pill itself carries
            // the border and focus ring; the TextField inside is chromeless.
            Rectangle {
                id: omnibox
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.verticalCenter: parent.verticalCenter
                height: Theme.size.control
                width: Math.min(Theme.size.searchWidth, parent.width)
                radius: Theme.radius.pill
                color: Theme.color.surfaceSunken
                border.width: search.activeFocus ? Theme.stroke.focus : Theme.stroke.hairline
                border.color: search.activeFocus ? Theme.color.focusRing : Theme.color.border

                Behavior on border.color {
                    ColorAnimation {
                        duration: Theme.motion.fast
                        easing.type: Theme.motion.standard
                    }
                }

                RowLayout {
                    anchors.fill: parent
                    // The pill's radius eats into its own corners, so the text
                    // starts a full gutter in while the round button only needs
                    // to clear the stroke.
                    anchors.leftMargin: Theme.spacing.md
                    anchors.rightMargin: Theme.spacing.xxs
                    anchors.topMargin: Theme.spacing.xxs
                    anchors.bottomMargin: Theme.spacing.xxs
                    spacing: Theme.spacing.xs

                    TextField {
                        id: search
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        background: null
                        leftPadding: 0
                        rightPadding: 0
                        color: Theme.color.text
                        placeholderTextColor: Theme.color.textFaint
                        font.pixelSize: Theme.typography.body.size
                        verticalAlignment: Text.AlignVCenter
                        placeholderText: qsTr("Filter files or tags")
                        onAccepted: files.refresh(root.selectedFolderId, text)
                    }

                    // Reload-style action: round, inside the pill, accent-filled.
                    Button {
                        id: scanButton
                        Layout.preferredWidth: Theme.size.hitTarget
                        Layout.preferredHeight: Theme.size.hitTarget
                        enabled: !root.backend.busy
                        Accessible.name: qsTr("Scan")
                        ToolTip.text: qsTr("Scan")
                        ToolTip.visible: hovered
                        onClicked: root.backend.scan()

                        background: Rectangle {
                            radius: Theme.radius.pill
                            color: scanButton.down ? Theme.color.accentPressed : scanButton.hovered ? Theme.color.accentHover : Theme.color.accent
                            opacity: scanButton.enabled ? 1.0 : Theme.opacity.disabled
                        }
                        contentItem: Label {
                            id: scanGlyph
                            text: "\u27f3"
                            color: Theme.color.accentFg
                            font.pixelSize: Theme.typography.subheading.size
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter

                            // Spin while a scan runs, so the button doubles as
                            // the busy indicator the way a browser reload does.
                            NumberAnimation on rotation {
                                running: root.backend.busy && !Theme.motion.reduce
                                from: 0
                                to: 360
                                duration: Theme.motion.slower * 4
                                loops: Animation.Infinite
                                // A value-source animation leaves the property
                                // wherever it stopped, so park it upright again.
                                onRunningChanged: if (!running)
                                    scanGlyph.rotation = 0;
                            }
                        }
                    }
                }
            }
        }
    }

    SplitView {
        Layout.fillWidth: true
        Layout.fillHeight: true

        // The gap between panes IS the handle: SplitView puts this delegate
        // between adjacent items, so its implicitWidth is the spacing. The
        // grip is a centred hairline that strengthens on hover/drag, leaving
        // the rest transparent so the page background shows through.
        handle: Rectangle {
            implicitWidth: Theme.spacing.md
            implicitHeight: Theme.spacing.md
            color: "transparent"

            // `SplitHandle` is the attached type carrying hover/press state.
            readonly property bool active: SplitHandle.hovered || SplitHandle.pressed

            Rectangle {
                anchors.centerIn: parent
                width: Theme.stroke.focus
                height: parent.height * 0.25
                radius: Theme.radius.pill
                color: parent.active ? Theme.color.accent : Theme.color.borderStrong
                opacity: parent.active ? 1.0 : Theme.opacity.muted

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.motion.fast
                        easing.type: Theme.motion.standard
                    }
                }
            }
        }

        // Left Pane
        Pane {
            SplitView.preferredWidth: Theme.size.sidebarWidth
            padding: Theme.spacing.md
            background: PanelBackground {}
            ColumnLayout {
                anchors.fill: parent
                spacing: Theme.spacing.sm
                Label {
                    text: qsTr("Folders")
                    color: Theme.color.text
                    font.pixelSize: Theme.typography.label.size
                    font.weight: Theme.typography.label.weight
                }
                Button {
                    Layout.fillWidth: true
                    implicitHeight: Theme.size.controlSm
                    text: qsTr("All files")
                    onClicked: {
                        root.selectedFolderId = -1;
                        files.refresh(-1, search.text);
                    }
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: folders
                    delegate: ItemDelegate {
                        id: folderDelegate
                        required property int id
                        required property string name
                        required property string path
                        width: ListView.view.width
                        height: Theme.size.control
                        text: name
                        font.pixelSize: Theme.typography.body.size
                        ToolTip.text: path
                        ToolTip.visible: hovered
                        background: Rectangle {
                            radius: Theme.radius.sm
                            color: folderDelegate.id === root.selectedFolderId ? Theme.color.selection : folderDelegate.hovered ? Theme.color.surfaceSunken : "transparent"
                        }
                        palette.text: folderDelegate.id === root.selectedFolderId ? Theme.color.selectionFg : Theme.color.text
                        onClicked: {
                            root.selectedFolderId = id;
                            files.refresh(id, search.text);
                        }
                    }
                }
            }
        }

        // Center Pane
        Pane {
            SplitView.fillWidth: true
            padding: Theme.spacing.md // Spacing to the previews from frame
            background: PanelBackground {}
            GridView {
                anchors.fill: parent
                clip: true
                model: files
                cellWidth: Theme.size.thumbnailMd + Theme.spacing.md
                cellHeight: Theme.size.thumbnailMd + Theme.spacing.xxl
                delegate: ItemDelegate {
                    id: fileDelegate
                    required property int id
                    required property string name
                    required property url thumbnailUrl
                    readonly property bool selected: fileDelegate.id === root.selectedFileId
                    width: Theme.size.thumbnailMd
                    height: Theme.size.thumbnailMd + Theme.spacing.xl
                    padding: Theme.spacing.xs
                    onClicked: root.selectedFileId = fileDelegate.id
                    background: Rectangle {
                        radius: Theme.radius.md
                        color: fileDelegate.selected ? Theme.color.selection : Theme.color.surfaceRaised
                        border.color: fileDelegate.selected ? Theme.color.accent : Theme.color.border
                        border.width: fileDelegate.selected ? Theme.stroke.focus : Theme.stroke.hairline
                    }
                    contentItem: Column {
                        spacing: Theme.spacing.xs
                        // Image {
                        //     width: parent.width
                        //     height: Theme.size.thumbnailMd - Theme.spacing.md
                        //     fillMode: Image.PreserveAspectFit
                        //     source: fileDelegate.thumbnailUrl
                        // }
                        Label {
                            width: parent.width
                            text: fileDelegate.name
                            color: fileDelegate.selected ? Theme.color.selectionFg : Theme.color.text
                            font.pixelSize: Theme.typography.caption.size
                            elide: Text.ElideMiddle
                            horizontalAlignment: Text.AlignHCenter
                        }
                    }
                }
            }
        }

        // Right Pane
        Pane {
            SplitView.preferredWidth: Theme.size.inspectorWidth
            padding: Theme.spacing.md
            background: PanelBackground {}
            ColumnLayout {
                anchors.fill: parent
                spacing: Theme.spacing.sm
                Label {
                    text: qsTr("Tags")
                    color: Theme.color.text
                    font.pixelSize: Theme.typography.label.size
                    font.weight: Theme.typography.label.weight
                }
                Label {
                    Layout.fillWidth: true
                    visible: tags.error.length > 0
                    color: Theme.color.danger
                    text: tags.error
                    wrapMode: Text.Wrap
                }
                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: Theme.spacing.xxs
                    model: tags
                    delegate: RowLayout {
                        id: tagRow
                        required property int id
                        required property string name
                        width: ListView.view.width
                        spacing: Theme.spacing.xxs
                        Label {
                            Layout.fillWidth: true
                            text: tagRow.name
                            color: Theme.color.text
                            font.pixelSize: Theme.typography.body.size
                        }
                        Button {
                            text: "+"
                            implicitWidth: Theme.size.hitTarget
                            implicitHeight: Theme.size.controlSm
                            enabled: root.selectedFileId >= 0
                            Accessible.name: qsTr("Assign %1").arg(tagRow.name)
                            onClicked: tags.assign(root.selectedFileId, tagRow.id)
                        }
                        Button {
                            text: "−"
                            implicitWidth: Theme.size.hitTarget
                            implicitHeight: Theme.size.controlSm
                            enabled: root.selectedFileId >= 0
                            Accessible.name: qsTr("Remove %1").arg(tagRow.name)
                            onClicked: tags.unassign(root.selectedFileId, tagRow.id)
                        }
                        Button {
                            text: "×"
                            implicitWidth: Theme.size.hitTarget
                            implicitHeight: Theme.size.controlSm
                            Accessible.name: qsTr("Delete %1").arg(tagRow.name)
                            onClicked: tags.remove(tagRow.id)
                            palette.buttonText: Theme.color.danger
                        }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacing.sm
                    TextField {
                        id: newTag
                        Layout.fillWidth: true
                        implicitHeight: Theme.size.control
                        placeholderText: qsTr("New tag")
                        onAccepted: {
                            tags.create(text);
                            clear();
                        }
                    }
                    Button {
                        text: qsTr("Add")
                        implicitHeight: Theme.size.control
                        enabled: newTag.text.trim().length > 0
                        onClicked: {
                            tags.create(newTag.text);
                            newTag.clear();
                        }
                    }
                }
            }
        }
    }
}
