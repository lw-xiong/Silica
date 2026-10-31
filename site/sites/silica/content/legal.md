---
title: Legal
layout: page
description: Silica is free software under the MIT license, from RAGTUX LLC. What the license means in plain words, what the warranty disclaimer means for your vault, trademark notice, privacy, and how to reach us.
---

# Legal

Silica is free software published by RAGTUX LLC, a Minnesota limited liability company.
This page says what that means in ordinary words. The words that actually bind are in
[LICENSE](https://github.com/ragtux/silica/blob/main/LICENSE) in the repository; where
this page and that file differ, the file governs.

## The license

Silica is under the **MIT license** — the whole program, every platform, every release.
In plain words:

- You may use it for anything, including commercially, without paying anyone.
- You may read, change and redistribute the source, and ship your own version.
- You may build it into something you sell.
- You must keep the copyright notice and the license text with any copy you pass on.
- It comes with **no warranty of any kind**.

There is no free tier, no paid tier, no license key, no activation and no account. There
is nothing to buy, so there is nothing to refund.

> **Warning** The no-warranty clause is not boilerplate here. Silica edits the files in your
> vault in place, it is version 0.3.2, and its writing half is unfinished. Keep a backup of
> any vault you open with it. If a bug of ours destroys a note, the license means the loss
> is yours — so please do not let a vault exist only inside an editor this young.

## Trademarks

**Obsidian** is a trademark of Dynalist Inc. Silica is an independent, clean-room
implementation that reads Obsidian's file formats; it is **not affiliated with, endorsed
by, or sponsored by** Obsidian or Dynalist Inc. The name is used here only to say truthfully
what Silica is compatible with.

Silica contains no code, stylesheets, icons or other assets from Obsidian's application
bundle. Its behavior was implemented from a written specification derived from observing
Obsidian's public behavior, its documented file formats and its public documentation.

**Silica** and **RAGTUX** are names used by RAGTUX LLC. The MIT license covers the code,
not the names: a fork is welcome, and should carry its own name rather than ours, so that
nobody mistakes your build for one of our releases.

## Third-party components

Silica is built on open-source libraries, chief among them
[GPUI](https://gpui.rs), the interface toolkit from the Zed editor. Each dependency keeps
its own license; the licenses travel with the source tree and are listed in
`Cargo.toml` and `Cargo.lock`. Nothing in the dependency set imposes a condition on your
use of Silica beyond what the MIT license already allows.

## Privacy

Silica is a program that reads a folder on your computer.

- It makes **no network connection of its own**. There is no telemetry, no analytics, no
  update check, no crash reporting and no license call.
- It has **no account** and collects nothing about you.
- Your notes are read from and written to your own disk, and nowhere else.

This website is static files served from a virtual server we run. It sets no cookies, runs
no analytics and has no third-party content, proxy or CDN in front of it. The server keeps
ordinary web-server logs — address, time, path, user agent — which are used to see that the
site works and are not shared. The company's full statement is at
[ragtux.com/privacy](https://ragtux.com/privacy/).

## Reaching us

| For | Address |
|---|---|
| Bugs, patches, feature discussion | [GitHub issues](https://github.com/ragtux/silica/issues) or [git.ragtux.com](https://git.ragtux.com/ragtux/silica/issues) |
| Anything else about Silica | [support@ragtux.com](mailto:support@ragtux.com) |
| Licensing and legal | [legal@ragtux.com](mailto:legal@ragtux.com) |
| Privacy | [privacy@ragtux.com](mailto:privacy@ragtux.com) |
| Security reports | [security@ragtux.com](mailto:security@ragtux.com) |

A bug report in public is usually worth more than an email, because the next person with
the same problem can find it.

## The company

RAGTUX LLC, Minnesota, USA. Company-level terms, privacy and support pages are at
[ragtux.com](https://ragtux.com/).
