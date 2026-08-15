# Hestia Library

Hestia indexes filesystem entries while preserving their locations and recognizing identical content.

## Language

**Watched Root**:
A user-selected directory tree whose UTF-8 regular files and real directories belong to the library. Watched roots neither overlap nor cross filesystem device boundaries.
_Avoid_: Watch path, library folder

**File Entry**:
A regular file visible at one path inside a watched location and the owner of that location's user metadata. Distinct file entries remain distinct even when their contents or filesystem object IDs are identical.
_Avoid_: File identity, asset

**Content Digest**:
A value derived only from a file's bytes. Equal content digests identify duplicate candidates, not the same file entry.
_Avoid_: Identity hash, file hash

**Filesystem Object ID**:
The device-and-inode pair identifying an underlying filesystem object on macOS and Linux. Multiple hard-linked file entries may share it.
_Avoid_: Identity hash, file ID

**Location**:
The path at which a file entry or folder is observed.
_Avoid_: Identity

**Duplicate Candidate**:
A file entry whose content digest equals that of another file entry. Both remain available until the user decides otherwise.
_Avoid_: Duplicate file

**Relocation**:
A change of location for the same file entry or folder, evidenced by an unchanged filesystem object ID. Movement across filesystems is removal followed by creation, not relocation.
_Avoid_: Content match

**Replacement**:
A new filesystem object observed at an existing location after the previous object disappears. The file entry and its location-owned user metadata remain, while its filesystem object ID and content digest change.
_Avoid_: Relocation
