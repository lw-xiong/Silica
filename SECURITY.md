# Security policy

## Reporting a vulnerability

Report security problems privately through GitHub's vulnerability reporting:
[github.com/ragtux/silica/security/advisories/new](https://github.com/ragtux/silica/security/advisories/new).
Do not open a public issue, pull request or discussion for one.

Please include the Silica version (or commit), your platform, how to reproduce
it — a vault, a note, a config file or a socket request that triggers it — and
what an attacker gains.

What to expect: an acknowledgement within 7 days, a fix in a tagged release,
and credit in its notes unless you would rather not be named. Please allow 90
days, or until the fix ships, before disclosing publicly; if a fix needs longer
you will hear why and we agree a date.

## What Silica promises

Silica opens a folder and treats it as the only thing it may touch. These are
the properties a report is measured against:

- **Writes stay inside the vault.** Every write goes through one checked path
  (`Vault::write_path`): a `..` or an absolute path is refused, so is a
  symbolic link anywhere along the way, and the file is opened without
  following a link in its place (`O_NOFOLLOW` on Unix). The only writes
  outside the folder are a file moved to the system trash when `trashOption`
  asks for it, and nothing else.
- **No network.** Silica makes no network connections. A remote image in a
  note is shown as a link, not fetched; a web link opens in the system browser
  only when clicked.
- **No code from the vault runs.** Community plugins, themes and CSS snippets
  are read as data (names, manifests, settings) and never executed or
  evaluated.
- **The control socket is local and private.** `--control <path>` binds a Unix
  domain socket with mode `0600`; it accepts no path that leaves the vault.
  It is off unless asked for, and it is as powerful as the keyboard — anyone
  who can connect to it can edit the vault.
- **Bytes it did not change are left alone.** Config files keep their unknown
  keys, BOMs and line endings; a note that changed on disk underneath an open
  buffer is not overwritten.

## Scope

In scope: the `silica` binary and its release archives, `silica-mcp`, the
control socket, and anything a malicious vault, note, config file, canvas or
link can make the app do — write outside the folder, run code, leak data,
crash with a buffer of unsaved edits.

Out of scope: problems that need an attacker who already controls your user
account or the vault folder's permissions, and the behaviour of Obsidian
itself.

## Supported versions

Only the latest tagged release gets fixes. The binaries are built by the
release workflows in this repository; see [README](README.md#releases) for how
to verify a download.
