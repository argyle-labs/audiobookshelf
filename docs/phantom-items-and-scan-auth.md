# Phantom library items, scan auth, and filing convention (operational KB)

Field notes from an audiobook library cleanup. Complements
[audiobookshelf.md](audiobookshelf.md).

## Phantom / duplicate library items

Two common causes:

1. **Companion files scanned as their own book** — e.g. a `.pdf` sitting next to
   the `.m4b` gets scanned as a separate "book."
2. **Stale rows** left behind after folder renames/moves.

Surgical fix (do this offline, and back up first):

1. Stop the container.
2. Back up `absdatabase.sqlite`.
3. Delete the phantom's rows across the related tables — `libraryItems`, `books`,
   `bookAuthors`, `bookSeries`, `mediaProgresses`.
4. Run `PRAGMA integrity_check;`.
5. Restart the container.

Prevention: keep non-audio companion files (PDFs, art) out of the item folder, or
confirm your scanner settings ignore them.

## You cannot fire a library scan remotely

Audiobookshelf authenticates with **rotating per-user JWTs**, not a static API key,
so a library scan cannot be triggered by an out-of-band/automated call. Rely on the
filesystem watcher for auto-detection, or the user must click **Scan** in the UI to
be certain new files are picked up. Plan importer/automation flows around this — do
not assume a remote scan trigger exists.

## Filing convention

Use `<Author>/<Title>/` for standalones and `<Author>/<Series>/NN - Title` for
series entries. Consistent filing keeps the scanner from creating split or
duplicate items and keeps series grouping correct.
