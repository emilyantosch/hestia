import QtQuick

ListModel {
    readonly property var _all: [
        { id: 10, name: "beach.jpg", path: "/2024/beach.jpg", type: "image", folderId: 1 },
        { id: 11, name: "mountains.png", path: "/2024/mountains.png", type: "image", folderId: 1 },
        { id: 12, name: "shot-01.png", path: "/Screenshots/shot-01.png", type: "image", folderId: 2 },
        { id: 13, name: "invoice-2024-03.pdf", path: "/Invoices/invoice-2024-03.pdf", type: "document", folderId: 3 },
        { id: 14, name: "a-really-long-file-name-that-should-be-elided.jpg", path: "/2024/long.jpg", type: "image", folderId: 1 }
    ]

    function refresh(folderId, search) {
        clear()
        const needle = (search || "").toLowerCase()
        for (const f of _all) {
            if (folderId >= 0 && f.folderId !== folderId) continue
            if (needle.length > 0 && !f.name.toLowerCase().includes(needle)) continue
            append({
                id: f.id, name: f.name, path: f.path, type: f.type,
                // Placeholder image; swap for any local file:// url to test real thumbnails.
                thumbnailUrl: Qt.resolvedUrl("placeholder.svg")
            })
        }
    }
}
