import QtQuick

ListModel {
    id: model
    property string error: ""
    property int _nextId: 100

    function refresh() {
        if (count === 0) {
            append({ id: 1, name: "family" })
            append({ id: 2, name: "travel" })
            append({ id: 3, name: "finance" })
        }
    }
    function create(name) {
        if (name.trim().length === 0) { error = qsTr("Tag name must not be empty"); return }
        error = ""
        append({ id: _nextId++, name: name.trim() })
    }
    function rename(tagId, name) {
        for (let i = 0; i < count; i++) if (get(i).id === tagId) { setProperty(i, "name", name); return }
    }
    // Shadows ListModel.remove(index); rebuild the model without the given tag.
    function remove(tagId) {
        const keep = []
        for (let i = 0; i < count; i++) if (get(i).id !== tagId) keep.push({ id: get(i).id, name: get(i).name })
        clear()
        for (const t of keep) append(t)
    }
    function assign(fileId, tagId) { console.log("mock assign", fileId, tagId) }
    function unassign(fileId, tagId) { console.log("mock unassign", fileId, tagId) }
}
