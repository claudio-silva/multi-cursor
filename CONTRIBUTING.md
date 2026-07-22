# Contributing to Multi Cursor

Thanks for considering a contribution. Bug reports, documentation improvements, design feedback, and pull requests are all welcome.

## Before you start

- Read the [README](README.md) for prerequisites and the local development workflow.
- Keep changes focused. If a change affects how environments, accounts, or Cursor data are managed, explain the intended behaviour and safety considerations in the pull request.
- Do not commit any data from `~/.multi-cursor/`; account snapshots can contain authentication tokens.

## Development workflow

Install dependencies and run the app in development mode:

```bash
npm install
npm run tauri dev
```

Build a production app with:

```bash
npm run tauri build
```

For local installation details, see the README. For signed release builds and DMGs, see [macOS distribution](docs/macos-distribution.md).

## Testing changes safely

Multi Cursor manages real Cursor data. Test account and environment changes with disposable Cursor accounts and environments where possible. In particular, check that:

- Cursor is closed before changes that rename folders or update authentication data.
- A normal Dock or Spotlight launch opens the environment selected in Multi Cursor.
- Removing an inactive environment sends its stored data to Trash.

## App icon

The branded icon source is [`assets/logo.png`](assets/logo.png). The main window uses a copy at `public/assets/logo.png`.

To regenerate the Tauri icon set (Dock, App Switcher, `.icns`, etc.):

```bash
npx tauri icon assets/logo.png
# Keep the in-app header icon in sync (128px is enough for the 48px UI slot):
sips -z 128 128 assets/logo.png --out public/assets/logo.png
```

Quit and relaunch Multi Cursor after changing icons; macOS caches Dock icons. A release build installed with `npm run install-app` is the most reliable way to check the final Dock appearance.
