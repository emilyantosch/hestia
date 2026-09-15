import QtQuick

ListModel {
    function refresh() {
        clear()
        append({ id: 1, name: "2024", path: "/home/me/Pictures/2024" })
        append({ id: 2, name: "Screenshots", path: "/home/me/Pictures/Screenshots" })
        append({ id: 3, name: "Invoices", path: "/home/me/Documents/Invoices" })
    }
}
