# Silica.app

macOS keeps an application's icon, name and version in a bundle, not in the executable, so the
release archive ships `Silica.app` and `bundle.sh` is what builds it:

```
just macos-app                                             # a release build, then the bundle
tools/macos-app/bundle.sh target/release/silica dist       # an existing binary
```

It lays out `Contents/MacOS/silica`, `Contents/Resources/silica.icns`,
`Contents/Info.plist` (from `crates/silica/resources/macos/Info.plist.in`, with the workspace
version filled in) and `Contents/PkgInfo`, then ad-hoc signs the result so macOS does not call a
bundle something else has touched damaged. The icon is cut by `tools/app-icons/cut.py`.

The ad-hoc signature is not notarization: a copy that arrives with the quarantine flag on it
still wants `right-click > Open` once, or `xattr -dr com.apple.quarantine Silica.app`. The script
runs anywhere — only the signing step needs a Mac, and it says so and carries on without one.

The archive also holds `silica` beside the bundle, a symlink to `Silica.app/Contents/MacOS/silica`,
so the documented `./silica ~/notes` keeps working.
