<p align="center">
  <img src="desktop/assets/logo.png" width="72" height="72" alt="CursorCue logo" />
</p>
<h1 align="center">CursorCue</h1>
<p align="center">Control the cursor others see while your mouse keeps working.</p>
<p align="center">Windows screen sharing for one-on-ones, team calls, reviews, walkthroughs, and presentations.</p>
<p align="center"><a href="https://cursorcue.pages.dev/">Visit the website</a></p>
<p align="center">
  <a href="https://github.com/sansynx/CursorCue/releases/download/v0.1.7/CursorCueSetup.exe">Download setup EXE</a> ·
  <a href="https://github.com/sansynx/CursorCue/releases/download/v0.1.7/CursorCue.msi">Download MSI</a> ·
  <a href="https://github.com/sansynx/CursorCue/releases/download/v0.1.7/CursorCue.exe">Download portable EXE</a>
</p>
<p align="center"><img src="docs/assets/website-hero.png" alt="CursorCue website hero with an interactive shared cursor preview" width="960" /></p>

## Downloads

Version **0.1.7** is available for Windows x64. The installers and portable executable are not code-signed.

| File                                                                                                   | Use                                                               |
| ------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------- |
| [CursorCueSetup.exe](https://github.com/sansynx/CursorCue/releases/download/v0.1.7/CursorCueSetup.exe) | Recommended installer with a setup guide and Start menu shortcut. |
| [CursorCue.msi](https://github.com/sansynx/CursorCue/releases/download/v0.1.7/CursorCue.msi)           | The same per-user installation through Windows Installer.         |
| [CursorCue.exe](https://github.com/sansynx/CursorCue/releases/download/v0.1.7/CursorCue.exe)           | Run directly without installation.                                |

Quit older CursorCue copies before installing or running this build. Settings are preserved when upgrading the installed app.

## How to use

1. Open CursorCue and choose the app window you want to share.
2. In your meeting app, choose **Share a window**, then select **CursorCue Share**.
3. Work in the original app. Freeze or hide affects the shared cursor while your real mouse keeps moving and clicking.
4. Open **Size and shortcuts** or **Tools → Cursor size & shortcuts** to change cursor size, appearance, and keys. Click **Apply** to save.

| Action        | Default shortcut | Behavior                                                |
| ------------- | ---------------- | ------------------------------------------------------- |
| Freeze        | Ctrl + Shift + F | Hold the shared cursor in place.                        |
| Hide / reveal | Ctrl + Shift + H | Hide or restore the shared cursor.                      |
| Resume        | Ctrl + Shift + R | Reconnect it to your real mouse.                        |
| Drop          | Ctrl + Shift + D | Place it at your real mouse position and hold it there. |
| Toggle        | Ctrl + Shift + G | Stop or restart the selected window capture.            |

If another app uses a shortcut, choose a different combination or clear its key to disable it. The Tools menu remains available. Closing the window leaves CursorCue in the tray; choose **Quit CursorCue** to exit.

## Requirements and current limits

- Requires Windows 11 x64 or Windows 10 version 2004 or newer.
- Keep the original app and CursorCue windows unminimized while sharing.
- Share the CursorCue window, not the original app or the entire desktop.
- Processing stays on your computer; the app has no account or cloud service.
- The current capture pipeline uses SDR BGRA8. HDR content can appear washed out.
- The cursor is inset slightly at frame edges to keep its whole shape visible.

## Build and check

The desktop app builds on Windows x64. Install:

- [Rust through rustup](https://rust-lang.org/tools/install/) with the stable `x86_64-pc-windows-msvc` toolchain, Rustfmt, and Clippy.
- [Visual Studio 2022 C++ Build Tools](https://learn.microsoft.com/en-us/windows/dev-environment/rust/setup) with **Desktop development with C++** and a Windows 10 or 11 SDK, including the x64 tools and libraries.
- [PowerShell 7](https://learn.microsoft.com/en-us/powershell/scripting/install/install-powershell-on-windows).
- For website work, [Node.js 24](https://nodejs.org/en/download) and [pnpm 10](https://pnpm.io/installation), matching CI. The website has no JavaScript runtime dependencies.

Open Developer PowerShell, clone the repository, and run the app. If contributing, clone your GitHub fork instead of the upstream repository shown here.

```powershell
git clone https://github.com/sansynx/CursorCue.git
cd CursorCue/desktop
cargo run -p cursorcue
```

Quit CursorCue through the Tools menu or tray before running the checks and packaging a release. From `desktop`:

```powershell
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace -- --test-threads=1
cargo build --locked --release --workspace
./installer/build-installer.ps1 -ExecutablePath ./target/release/cursorcue.exe -OutputDir ../release
```

The icon is already included. Regenerate it with `./assets/build-icon.ps1` when changing the logo. Native UI and capture checks need a Windows desktop session.

The Cargo workspace version controls executable and installer version metadata. Each release gets a distinct product identity within the same upgrade family. The Windows CRT is linked statically. Published packages come from CI with local build paths removed.

```powershell
cd ../website
pnpm install --frozen-lockfile
pnpm run check
pnpm run test
pnpm run dev
```

Open `http://127.0.0.1:4173/`. Website download links point to the GitHub release assets.

Cloudflare Pages publishes [cursorcue.pages.dev](https://cursorcue.pages.dev/) from `website/dist` on `main`.

The native executable also supports an isolated capture diagnostic:

```powershell
../desktop/target/release/cursorcue.exe --diagnostic --seconds=10
```

Installer lifecycle tests must use isolated product, upgrade, and component GUIDs and a matching test wrapper. The test script rejects production identities and mismatched payloads.

## Contributing

1. Fork the repository on GitHub and clone your fork.
2. Create a branch for your change with `git switch -c your-change`.
3. Make the change and run the relevant checks above. Keep changes focused and include a regression test for behavior fixes.
4. Push the branch to your fork and open a pull request against `sansynx/CursorCue:main`. Describe the change and the checks you ran. For capture or UI changes, include the Windows version, display scaling, and any multi-monitor checks.

Contributions require owner review and passing website, Windows, installer, and secret checks. External contributors' CI runs require maintainer approval. GitHub Actions runs with read-only permissions. Only the repository owner can bypass the pull request checks; force pushes and branch deletion are blocked. The checks are defined in [the CI workflow](.github/workflows/ci.yml).

### Where to make changes

| Path                                                                               | Responsibility                                                                                |
| ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| [desktop/app/src](desktop/app/src)                                                 | Windows app lifecycle, tray, shortcuts, main window, and settings UI.                         |
| [cursorcue-core](desktop/crates/cursorcue-core/src/lib.rs)                         | Cursor state, freezing, visibility, smoothing, and coordinates.                               |
| [cursorcue-config](desktop/crates/cursorcue-config)                                | Settings validation, persistence, recovery, and configuration tests.                          |
| [cursorcue-platform-windows](desktop/crates/cursorcue-platform-windows/src/lib.rs) | Windows Graphics Capture sessions and frame delivery.                                         |
| [cursorcue-render](desktop/crates/cursorcue-render/src)                            | Direct3D 11 compositing, cursor drawing, and the HLSL shader.                                 |
| [desktop/installer](desktop/installer)                                             | MSI builder, setup wrapper, and isolated install/uninstall tests.                             |
| [website](website)                                                                 | Authored HTML, CSS, and JavaScript in `dist`, the local server, and browser simulation tests. |

Cargo build output in `target/`, local caches, credentials, installers, and temporary files are ignored. Keep `desktop/Cargo.lock`, `website/pnpm-lock.yaml`, and `desktop/.cargo/config.toml` committed so contributors use consistent dependencies and build settings.

For a bug report, [open an issue](https://github.com/sansynx/CursorCue/issues) with steps to reproduce, expected and actual behavior, the app version, Windows version, and relevant display or meeting-app details. Remove private content from screenshots and logs.

## License

[MIT](LICENSE)
