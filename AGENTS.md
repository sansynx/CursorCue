# CursorCue coding agent guide

## Scope and working approach

This file applies to the entire repository. Read it before editing. Follow any more specific `AGENTS.md` within the files' directory tree, and respect the user's task and the agent's governing instructions.

- Check `git status` first and preserve unrelated work.
- Read the relevant source, manifests, tests, and [CI workflow](.github/workflows/ci.yml) before making changes. Use `rg` for focused searches.
- Prefer the smallest correct change within the existing architecture. Do not add frameworks, services, dependencies, or unrelated refactors for convenience.
- Keep this guide accurate when changing documented commands, paths, or behavior. Use [README.md](README.md) for human onboarding and [LICENSE](LICENSE) for the MIT license.
- Keep explanations concise. Report what changed, which checks actually ran, their results, and any unverified behavior. Do not use em dashes in text you add.

## Product and platform

CursorCue is a Windows app that lets users freeze, hide, resume, or reposition the cursor others see while their real mouse keeps working. It supports one-on-ones, team calls, reviews, walkthroughs, and presentations. Do not describe it as a presentation-only product.

- The desktop app supports Windows x64, Windows 11 or Windows 10 build 19041 and newer. Do not add another platform without an explicit request.
- User-facing names are **CursorCue** and **CursorCue Share**. Internal names such as `presentation.rs` are implementation details and do not require cosmetic renaming.
- Users choose an app window, share the **CursorCue Share** window in their meeting app, and continue working in the original app. Both windows must remain unminimized.
- Capture and cursor processing stay local. The app has no account, backend, telemetry, or cloud upload service. Preserve that behavior.
- The capture pipeline uses SDR BGRA8. HDR support requires a deliberate pipeline change and validation.

## Repository map

| Path                                                  | Responsibility                                                                                                     |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `desktop/Cargo.toml`                                  | Rust workspace, shared version, Rust edition, and release profile.                                                 |
| `desktop/.cargo/config.toml`                          | Windows MSVC flags, including static CRT linkage.                                                                  |
| `desktop/app/src/main.rs`                             | Windows entry point and startup error reporting.                                                                   |
| `desktop/app/src/native.rs`                           | Window message loop, app lifecycle, tray, source selection, hotkeys, capture/render coordination, and diagnostics. |
| `desktop/app/src/presentation.rs`                     | Welcome instructions and controls for the share window.                                                            |
| `desktop/app/src/settings.rs`                         | Native settings controls, validation feedback, DPI layout, and scrolling.                                          |
| `desktop/app/build.rs` and `desktop/app/app.manifest` | App icon/version resources and Windows manifest.                                                                   |
| `desktop/crates/cursorcue-core`                       | Cursor state, movement, smoothing, and coordinate mapping.                                                         |
| `desktop/crates/cursorcue-config`                     | Configuration validation, atomic persistence, recovery, and schema compatibility.                                  |
| `desktop/crates/cursorcue-platform-windows`           | Windows Graphics Capture sessions, frame delivery, and capture resource ownership.                                 |
| `desktop/crates/cursorcue-render`                     | Direct3D 11 composition, cursor geometry, GPU resources, and `src/composite.hlsl`.                                 |
| `desktop/installer`                                   | MSI generation, native setup wrapper, manifests, and isolated lifecycle tests.                                     |
| `desktop/assets`                                      | Canonical logo, desktop icon, and the icon generation script.                                                      |
| `website/dist`                                        | Authored HTML, CSS, JavaScript, logo, and favicon. This directory is source and must remain tracked.               |
| `website/server.mjs`                                  | Local static server, bound to `127.0.0.1`, with default port 4173.                                                 |
| `website/tests`                                       | Node tests for the browser cursor simulation and smooth scrolling.                                                 |
| `docs/assets`                                         | The website hero screenshot used by the README.                                                                    |
| `.github`                                             | CI and code ownership. Repository rulesets are configured in GitHub settings.                                      |

The runtime flow is selected HWND to Windows Graphics Capture, then GPU texture composition with the synthetic cursor, then the CursorCue share window. The core crate owns cursor behavior, the config crate owns persisted settings, and the native app coordinates them.

## Development setup and commands

Use Windows x64, PowerShell 7, stable Rust with the `x86_64-pc-windows-msvc` toolchain, Rustfmt, Clippy, Visual Studio 2022 C++ tools, and a complete Windows SDK. See the installation links in [README.md](README.md#build-and-check). Website work uses Node.js 24 and pnpm 10, matching CI.

Commands below start from the stated directory. Do not rely on task-specific toolchain paths, personal directories, or installed release binaries.

### Desktop

From `desktop`:

```powershell
cargo +stable run --locked -p cursorcue
```

Quit the running app before replacing its executable. For desktop changes, use the relevant checks:

```powershell
cargo +stable fmt --all --check
cargo +stable clippy --locked --workspace --all-targets -- -D warnings
cargo +stable test --locked --workspace -- --test-threads=1
cargo +stable build --locked --release --workspace
```

For a focused core change, start with `cargo +stable test --locked -p cursorcue-core`. Native tests interact with real Windows controls and resources, so keep the full suite single-threaded and use a Windows desktop session.

After a release build, run the isolated capture diagnostic from `desktop` when capture, rendering, or lifecycle behavior changes:

```powershell
./target/release/cursorcue.exe --diagnostic --seconds=10
```

The diagnostic creates its own source window, avoids production settings and hotkeys, and checks captured GPU pixels and teardown. Check its exit code. Its result does not establish compatibility with every meeting app or display setup.

### Website

From `website`:

```powershell
pnpm install --frozen-lockfile
pnpm run check
pnpm run test
pnpm run dev
```

Open `http://127.0.0.1:4173/`. The website is plain HTML, CSS, and JavaScript with no runtime dependencies. There is no bundler, React app, or `build` script. Do not invent lint, typecheck, or build commands that are absent from `package.json`.

Use Prettier 3.6.2 for supported files, matching CI. From `website`, the full formatting check is:

```powershell
pnpm dlx prettier@3.6.2 --check ../AGENTS.md ../README.md ../.github/workflows/ci.yml package.json server.mjs "dist/*.html" "dist/*.css" "dist/*.js" "tests/*.mjs"
```

For small changes, format and inspect only affected files. Verify layout and interaction changes in the browser, including narrow viewports, keyboard access, and reduced-motion behavior when relevant. The browser demo is a simulation and must be described accordingly.

## Runtime constraints to preserve

- Keep the physical mouse independent of the synthetic shared cursor. Windows Graphics Capture must exclude the physical cursor.
- Use consistent physical coordinates across monitor origins and DPI scaling, including negative monitor coordinates. Keep the entire synthetic cursor visible at source edges and after resizes.
- Preserve the settings window across show, hide, reopen, and DPI changes. Controls, labels, and action buttons must remain reachable without clipping. Welcome text must not move in response to unnecessary wheel input.
- Treat shortcut registration and settings persistence as a transaction. On a conflict or save failure, restore the previous working bindings. Allow disabled shortcuts and keep menu controls available without repeated modal prompts.
- Closing the main window normally leaves the app in the tray. Preserve tray recovery after Explorer restarts, and keep the main window accessible if the tray cannot be registered.
- Keep HWND, COM, GDI, capture callbacks, textures, and swap-chain lifetimes explicit. Maintain the correct thread/apartment ownership and teardown order. Keep useful `SAFETY` comments that explain these contracts.
- Preserve GPU texture processing and conditional rendering. Avoid full-frame CPU readbacks, hot-path allocation, busy loops, and unnecessary presentation of static content. Diagnostic pixel probes are test tools.
- Configuration lives at `%APPDATA%\CursorCue\config.json`. Preserve validation, atomic replacement, corrupt-file backups, and protection of newer schema versions. Tests must use temporary configuration paths.

## Installer and release work

From `desktop`, build production packages only when the task requires packaging:

```powershell
./installer/build-installer.ps1 -ExecutablePath ./target/release/cursorcue.exe -OutputDir ../release
```

The builder discovers installed C++ tools and selects a complete Windows SDK. Preserve that discovery, version checks, per-user installation, production upgrade identity, icon resources, and manifest behavior.

Installer lifecycle tests must use fresh product, upgrade, and component GUIDs, plus a setup wrapper built from the same test MSI. Never test installation or removal against the production CursorCue family. From `desktop`:

```powershell
$testProduct = [Guid]::NewGuid().ToString('B')
$testUpgrade = [Guid]::NewGuid().ToString('B')
$testComponent = [Guid]::NewGuid().ToString('B')
./installer/build-installer.ps1 -ExecutablePath ./target/release/cursorcue.exe -OutputDir ../work/installer-agent -ProductCode $testProduct -UpgradeCode $testUpgrade -ComponentCode $testComponent
./installer/test-installer.ps1 -MsiPath ../work/installer-agent/CursorCue.msi -ExecutablePath ./target/release/cursorcue.exe -SetupPath ../work/installer-agent/CursorCueSetup.exe
```

Do not bypass the test script's identity, payload, or directory checks. Use a fresh fixture after a failed run, and inspect any remaining test installation before retrying.

Published packages come from the successful CI build for the release commit. CI uses an explicit x64 target, removes local build paths, checks package privacy, and uploads Windows artifacts. Its executable path is `desktop/target/x86_64-pc-windows-msvc/release/cursorcue.exe`, unlike the local host-build path above. Keep installer and artifact paths aligned when changing CI.

- Read the workspace version from `desktop/Cargo.toml`. Keep executable, MSI, setup wrapper, README, and website release versions consistent when preparing a release.
- Do not rewrite published tags or publish untested binaries. Build success and code signing are separate facts; current packages are unsigned.
- For logo changes, run `desktop/assets/build-icon.ps1` on Windows. It updates `desktop/assets/CursorCue.ico`, `website/dist/logo.png`, and `website/dist/favicon.ico` from the canonical `desktop/assets/logo.png`.
- Keep the website's existing minimal design, shared branding, readable instructions, white initial navbar, and blue navbar after scrolling.

## Change quality and validation

- Keep pure cursor behavior in `cursorcue-core`, persistence in `cursorcue-config`, capture in the Windows platform crate, rendering in the render crate, and UI coordination in the app.
- Follow Rustfmt and existing Rust 2024 patterns. Do not expand the unsafe surface or add abstractions without a concrete need.
- Use pnpm for JavaScript work. Preserve `website/pnpm-lock.yaml` and do not create `package-lock.json`. Check the existing stack before proposing a dependency.
- Add regression coverage for behavior changes. Test observable results, including failure handling, rather than duplicating implementation details. Do not add tests solely for prose changes.
- Use proportional validation. Docs changes need formatting and link/path checks. Rust changes need relevant tests and formatting; substantial native changes also need Clippy, release build, and runtime verification. Installer changes need isolated lifecycle checks. Website changes need relevant scripts and browser verification.
- For native UI or capture changes, check the affected behavior at different display scaling, near source edges, across monitors, and through resize, close, stop, and restart paths as applicable. State any unavailable manual checks.
- Fix failures caused by the change. Report unrelated failures or missing tools without claiming checks passed. Do not weaken CI, remove assertions, or skip tests to obtain a green result.

## Security, artifacts, and publishing

- Never commit credentials, private keys, personal paths, real user configuration, private screen content, or sensitive logs. Do not log secret values.
- Keep `desktop/Cargo.lock`, `website/pnpm-lock.yaml`, and `desktop/.cargo/config.toml` tracked. Respect `.gitignore` for `target`, `node_modules`, local tool caches, work files, and release artifacts.
- Verify resolved paths before recursive deletion or moving directories. Keep temporary work in the checkout's ignored `work` directory or the task's designated workspace. Preserve unrelated user files.
- GitHub Actions must retain read-only permissions, full commit SHA pins, and secret scanning. Do not expose secrets to untrusted pull requests or introduce privileged PR workflows.
- Contributors use a branch and a pull request to `main`, with owner review and passing **Website checks**, **Windows checks**, and **Secret scan**. External contributors' workflow runs need maintainer approval. Only the owner has the configured PR bypass; this does not authorize an agent to use it without task authorization. Force pushes and main-branch deletion are blocked.
- Publishing releases, pushing changes, changing repository settings, or deploying requires authorization within the task. Do not weaken repository rules to finish a change.
- The website is hosted on Cloudflare Pages at <https://cursorcue.pages.dev/>. Its Git integration deploys `website/dist` from `main`. Use Pages for this static site. For authorized Cloudflare CLI work, use `cf` unless the project gains a Wrangler configuration. Discover commands with an anonymous `cf cli search` query, inspect the chosen command's help, and use `cf schema` for request details. Keep tokens and account details out of source and command output.

Before finishing, inspect the final diff and working tree, confirm the requested behavior, and report the checks that actually ran. Update existing documentation when the change alters its instructions; do not create extra planning or progress documents unless requested.
