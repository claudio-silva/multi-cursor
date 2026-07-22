# macOS distribution

This guide is for maintainers who want to distribute Multi Cursor beyond their own Mac. The repository intentionally defaults to a local `.app` build; this keeps normal development and local installation free from the duplicate Spotlight entries that disk-image builds can create.

For a personal installation, use the [README](../README.md#install) instead.

## Build targets

`src-tauri/tauri.conf.json` currently sets `bundle.targets` to `["app"]`. This produces only a macOS app bundle:

```text
src-tauri/target/release/bundle/macos/Multi Cursor.app
```

The local `npm run install-app` workflow copies that bundle to `~/Applications` and removes local build artifacts under `src-tauri/target/`. Do not run it after producing a DMG you want to keep.

## Create a DMG

For a distribution build, temporarily change the bundle targets in `src-tauri/tauri.conf.json`:

```json
"bundle": {
  "targets": ["app", "dmg"]
}
```

Build the release:

```bash
npm run tauri build
```

The artifacts normally appear here:

- `src-tauri/target/release/bundle/macos/Multi Cursor.app`
- `src-tauri/target/release/bundle/dmg/*.dmg`

Copy the DMG to a release or CI-artifacts directory before running `npm run install-app`; the install script intentionally deletes `.app` and `.dmg` files under the target tree.

Disk-image builds mount temporary volumes under `/Volumes`. macOS can register these temporary locations in Launch Services or Spotlight, which is why the repository does not enable DMG output by default.

## Sign and notarize

Unsigned or ad-hoc-signed apps are appropriate only for the Mac on which they were built. To distribute to other people without Gatekeeper warnings, join the Apple Developer Program, sign the app with a **Developer ID Application** certificate, and notarize it.

1. Create a Developer ID Application certificate in the Apple Developer portal and install it in Keychain Access.
2. Create an app-specific Apple ID password for notarization.
3. Configure Tauri's signing and notarization settings. The exact configuration changes with Tauri releases, so follow the current [Tauri macOS code-signing documentation](https://v2.tauri.app/distribute/sign/macos/).
4. Confirm that macOS can see the signing identity:

   ```bash
   security find-identity -v -p codesigning
   ```

5. Build the signed release:

   ```bash
   npm run tauri build
   ```

6. Verify the app before publishing:

   ```bash
   codesign -dv --verbose=4 "src-tauri/target/release/bundle/macos/Multi Cursor.app"
   spctl -a -vv "src-tauri/target/release/bundle/macos/Multi Cursor.app"
   ```

   If you built a DMG, verify that too:

   ```bash
   spctl -a -t open -vv --context context:primary-signature path/to/MultiCursor.dmg
   ```

Distribute a notarized DMG or a ZIP archive of the signed `.app`. People installing it can drag the app into `/Applications` or `~/Applications`.

## Release checklist

- Build from a clean, tested revision.
- Sign and notarize with a Developer ID certificate.
- Verify the app and DMG with `codesign` and `spctl`.
- Test the artifact on a Mac that did not build it.
- Publish only the notarized artifact, not anything under a local `src-tauri/target/` directory.
