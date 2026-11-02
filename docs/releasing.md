# Releasing Prism

Every push to the `release` branch runs `.github/workflows/release.yml`. The workflow builds
signed installers for macOS and Windows, plus a FreeBSD tarball, then creates
or updates a **draft** GitHub Release named `Prism v<version>`. Nobody sees a draft until a
maintainer publishes it.

This is Prism's implementation of the shared
[release playbook](release-playbook.md). User-facing names say **Prism**. Files, binaries
and ids stay lowercase (`prism-<version>-<platform>-<arch>.<ext>`, `com.nkstudio.prism`).

## Cutting a release

1. **Bump the version** on `main`. The only place it lives is `[workspace.package] version`
   in the root `Cargo.toml`:

   ```sh
   cargo xtask version                 # prints the current version, e.g. 0.1.0
   cargo xtask version set 0.2.0       # or 0.2.0-rc.1; updates Cargo.toml and Cargo.lock
   ```

   Commit the change (`Cargo.toml` + `Cargo.lock`) through the normal review flow.
2. **Merge `main` into `release`** (or fast-forward it) and push. The workflow starts by itself.
3. **Wait for the draft.** After about 30 to 45 minutes (notarization is the slow part), the
   Releases page has a draft `Prism v0.2.0`, tagged `v0.2.0` on the pushed commit, with
   every artifact and `SHA256SUMS.txt`. The notes are generated from the merged PRs.
4. **Check it.** Download an installer or two and look at the job summaries. Any
   `::warning::` there means a signing secret was missing and that artifact is unsigned.
5. **Add the scorecard deltas** to the draft's notes: what moved in
   [`docs/scorecard.md`](scorecard.md) since the previous release. Compare the summary table
   and the performance table of both tags (`git diff v<previous> v<this> -- docs/scorecard.md`)
   and list each area's done / partial / missing change, the scenarios that newly meet their
   budget or got slower, corpus floors raised, and the change in settings that do nothing.
   Quote numbers with the baseline's machine and load average.
6. **Publish** the draft in the GitHub UI. Publishing creates the `v0.2.0` tag. Versions with a
   pre-release suffix (`-rc.1`) are marked as pre-releases.

Pushing to `release` again before you publish rebuilds the same draft and replaces its assets.
After the draft is published, the workflow refuses to touch that version again, so bump it first.

**Test runs:** *Actions → Release → Run workflow* runs the whole pipeline by hand. The optional
`version` input (such as `0.2.0-rc.1`) overrides `Cargo.toml` for that run only. The jobs apply
it with `cargo xtask version set` before building, so the binaries report it too. The run still
needs the `release` environment, which only the `release` branch can use, so pick that branch
in the dialog.

## What gets built

| Platform | Artifacts | Built on |
|---|---|---|
| macOS 11+ (universal: Apple silicon + Intel) | `prism-<v>-macos-universal.dmg`, `prism-cli-<v>-macos-universal.zip` | `macos-15` |
| Windows 10+ x64 | `prism-<v>-windows-x64.msi`, `prism-<v>-windows-x64-portable.zip` | `windows-latest` |
| Windows 10+ x86 (32-bit) | `prism-<v>-windows-x86.msi`, `prism-<v>-windows-x86-portable.zip` | `windows-latest` |
| FreeBSD 14 x86_64 | `prism-<v>-freebsd-x86_64.tar.gz` | FreeBSD 14.3 VM on `ubuntu-latest` |

Every binary reports its version, the commit and the build date: `prism --version`,
`prism-cli --version`, and *Help › About Prism*. CI sets `PRISM_BUILD_SHA` and
`PRISM_BUILD_DATE`, and `crates/engine/src/build_info.rs` reads them at compile time. A plain
`cargo build` doesn't set them and reports `0.2.0 (dev build)`.

Every desktop build job (macOS, Windows, FreeBSD) also checks out [craft-fonts](https://github.com/storytold/craft-fonts)
at the commit in `CRAFT_FONTS_REF` (top of `release.yml`) and builds with `CRAFT_FONTS_DIR` and
`CRAFT_FONTS_REQUIRED=1`, so desktop releases embed its Japanese fonts and fail rather than ship without them. The packages carry each font's licence as
`OFL-<family>.txt` (`copy_font_licences` in `packaging/env.sh`; the portable zip on Windows;
`Contents/Resources/Licenses` in the macOS app). Bump the pin deliberately, together with the one
in `ci.yml`. Rules: `../craftrules/standards/fonts.md`; build option: `docs/development.md` › Fonts.

### macOS

`packaging/macos/package.sh` builds `aarch64-apple-darwin` and `x86_64-apple-darwin` with
`MACOSX_DEPLOYMENT_TARGET=11.0`, joins them with `lipo`, and assembles `Prism.app`:

- `Info.plist` is generated from `Info.plist.in`. The bundle id is `com.nkstudio.prism`.
  The plist sets `LSMinimumSystemVersion` 11.0, `NSHighResolutionCapable`, and document types:
  `.prism` (Owner), plus PSD/PSB and the image formats Prism reads (Alternate, so it never
  takes over Preview's defaults). The icon is `assets/app-icon/prism.icns`.
- **Signing** goes inside-out with the hardened runtime and a secure timestamp. The executable
  is signed first, then the bundle. There's no `--deep` on the final signature. The
  entitlements (`entitlements.plist`) are deliberately empty.
- **Notarization:** the app is zipped and sent with `xcrun notarytool submit --wait`, then the
  ticket is stapled to the app. The app goes on a DMG (`hdiutil`, with an `Applications` link
  to drag onto). The DMG is signed, notarized and stapled too. The script checks the results
  with `codesign --verify --strict`, `stapler validate` and `spctl -a -vvv`.
  Its Finder window (background, icon size and positions) comes from
  [`packaging/macos/dmg/`](../packaging/macos/dmg/README.md), and its volume is named `Prism`
  without the version, which the window's background needs; the DMG file name keeps the version.
- **CLI:** the universal `prism-cli` is signed with the same Developer ID, the hardened
  runtime and a secure timestamp (identifier `com.nkstudio.prism-cli`), zipped, and the zip
  is sent to `notarytool`. Only `.app`, `.dmg` and `.pkg` can hold a stapled ticket, not a bare
  Mach-O, so Gatekeeper looks the CLI's ticket up online the first time a downloaded
  (quarantined) copy runs. Offline, that first run can be refused until the Mac is online again.
- **Verification:** `packaging/macos/verify.sh` checks the artifacts as users download them. It
  unpacks the CLI zip and requires, for the binary inside, `codesign --verify --strict`, both
  architectures, the hardened runtime flag, a `Developer ID Application` authority from
  `APPLE_TEAM_ID`, a timestamp, and `spctl --assess --type install` reporting
  `source=Notarized Developer ID` (the same online lookup Gatekeeper does). For the DMG it runs
  `codesign --verify`, `stapler validate` and `spctl --type open`. The release workflow runs it
  as its own step after packaging. With signing secrets it fails the job on any miss; without
  them it only checks signature integrity and warns, like `package.sh`. Run it locally after a
  build: `packaging/macos/verify.sh --arch aarch64`.

Locally, without certificates, the script signs ad-hoc (`codesign -s -`) and skips notarization.
That's enough to check the bundle and the DMG on your own Mac:

```sh
packaging/macos/package.sh                    # universal; needs both rustup targets
packaging/macos/package.sh --arch aarch64     # quicker, host-only
open dist/release/prism-*-macos-*.dmg
```

### Windows

`packaging/windows/package.ps1 -Arch x64|x86` builds with `-C target-feature=+crt-static`.
The static C runtime means neither the MSI nor the portable zip needs the Visual C++
redistributable, which matters for a standalone installer and costs only about 100 KB. The flag
goes in `CARGO_TARGET_<TRIPLE>_RUSTFLAGS`, so host build scripts aren't affected.

- `apps/prism/build.rs` and `apps/prism-cli/build.rs` embed the icon
  (`assets/app-icon/prism.ico`) and VERSIONINFO with the `winresource` crate
  (build dependency `0.1`). They only do this when targeting Windows. Elsewhere each
  script is a no-op, even if `PRISM_REQUIRE_WINRES` is set. Both set ProductName to Prism, CompanyName to
  NK-Studio and the original copyright line. The GUI description is
  "Prism image editor" (`prism.exe`). The CLI description is
  "Prism command-line interface", with OriginalFilename `prism-cli.exe`
  and InternalName `prism-cli`. Version strings are the Cargo package version,
  so a pre-release tag is kept; the numeric file and product versions are
  winresource's `major.minor.patch.0`. A missing resource compiler warns, so an
  optional cross-compile still links. `package.ps1` sets `PRISM_REQUIRE_WINRES=1`.
  The CLI build script then returns an error naming `rc.exe` or `windres` and does
  not panic.
- Release builds use the GUI subsystem, so Start Menu launches don't open a console window.
  The CLI stays console subsystem 3 so its output reaches the terminal.
- `prism.wxs` (WiX v5) is a per-machine install into Program Files with an advertised
  Start Menu shortcut. Prism becomes the default app for `.prism` and is listed under
  "Open with" for PSD/PSB and image files. It also registers App Paths (Win+R `prism`).
  The MSI version is the numeric `X.Y.Z`, because MSI has no pre-release field. Same-version
  upgrades are allowed so that release candidates replace each other.
- Shortcut icon identifiers keep the executable's `.exe` extension: MSI uses the identifier
  as the cached icon filename, and an extensionless filename can render as a blank document
  icon. `packaging/windows/check-icons.ps1` checks the references and extensions in CI;
  `package.ps1` also validates the built MSI with ICE50 before signing it.
- **Portable zip:** it ships `packaging/windows/portable.txt` beside `prism.exe`. That
  marker (or a `Prism.portable` file) switches on portable mode: preferences, presets,
  recovery autosaves and the GPU startup marker go to `PrismData\` next to the exe instead
  of `%APPDATA%\Prism`. If that folder isn't writable the app warns and uses `%APPDATA%`.
  The MSI has no marker. The logic is in `apps/prism/src/app_dirs.rs` and works the same
  on macOS.
- **Signing:** `packaging/windows/sign.ps1` signs both `.exe` files and then the `.msi` with
  `signtool`, using SHA-256 and an RFC 3161 timestamp. It uses whichever material is present:
  1. a `.pfx` file (`WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`), timestamped by
     DigiCert, or
  2. Azure Trusted Signing (`AZURE_*`): the script downloads the
     `Microsoft.Trusted.Signing.Client` dlib and timestamps with Microsoft's server.

  The script is the single place to change when the Windows signing setup changes.

Locally on Windows: `dotnet tool install -g wix --version 5.0.2`, then
`pwsh packaging/windows/package.ps1 -Arch x64`.

### FreeBSD

GitHub has no FreeBSD runners, so the `freebsd` job runs `packaging/freebsd/package.sh` in a
FreeBSD 14.3 VM (`vmactions/freebsd-vm`, pinned by commit), with the same packages as the
FreeBSD CI workflow (`.github/workflows/freebsd.yml`) plus `bash`. The script builds both
binaries with `CARGO_BUILD_JOBS=4` (more runs the 12 GB VM out of memory) and writes
`prism-<v>-freebsd-x86_64.tar.gz`. FreeBSD's `uname -m` says `amd64`; the file name uses
`x86_64` like the other artifacts. The tarball is a `/usr/local`-style tree: `bin/prism`,
`bin/prism-cli`, and under `share/` the same desktop entry, MIME type, AppStream metainfo
and hicolor icons from `packaging/freebsd`, plus the licences, `NOTICE` and `ATTRIBUTION.md` in
`share/doc/prism/`. Users install it with:

```sh
pkg install libxkbcommon wayland libX11 libXcursor libXrandr libXi libxcb mesa-libs vulkan-loader gtk3 fontconfig freetype2 alsa-lib
tar -xzf prism-<v>-freebsd-x86_64.tar.gz --strip-components 1 -C /usr/local
```

The job signs nothing, so it runs outside the `release` environment and gets no secrets. The
version, build date and commit reach the VM through the action's `envs:` (by name, never
templated into the script), and the build happens in `/var/tmp` so only `dist/` is copied back
out of the VM. The binaries aren't signed: FreeBSD has no code-signing scheme for loose
binaries, so check the tarball against `SHA256SUMS.txt`.

The FreeBSD CI workflow runs the same script after its build and test (on pushes to `main`, and
on PRs that touch `packaging/freebsd/`, `freebsd.yml` or `release.yml`), so the release job
isn't the first place it runs. Locally, on any OS, `packaging/freebsd/package.sh --dry-run` stages the tree from stub binaries
and lists the tarball, which checks the layout without a FreeBSD machine.

## In-app updates

Installed copies find new versions through a signed feed on a Vercel site (`crates/updater`,
`crates/ui-egui/src/update_ui.rs`). Help › Check for Updates… checks on demand. With
*Preferences › Integrations › Check for updates* on (the default), Prism also checks once a day
at launch. It announces a new version unless the user chose *Skip This Version*.

**How it stays safe.** `stable/latest.json` lists each platform's package with its URL, size and
SHA-256. It is signed with a minisign (Ed25519) key whose public half is compiled into Prism, so a
compromised site can't make Prism install anything else. Downloads are checked against the signed
size and hash. On macOS the new `Prism.app` must also pass `codesign --verify --deep --strict` and
carry the same Team ID. The whole bundle is swapped after Prism quits (two renames, rolled back on
failure). Prism never patches files inside it, which would break the signature. On Windows the
MSI (a major upgrade) is run with `msiexec /passive` after Prism quits. Portable copies, FreeBSD
and copies that can't write their own folder get a download link instead.

**Site layout** (the Vercel project, a separate repository):

```text
stable/latest.json            the feed (never cached; vercel.json is written by the publish step)
stable/latest.json.minisig    its signature
releases/<version>/           update packages: prism-<v>-macos-<arch>.app.zip, prism-<v>-windows-<arch>.msi
```

Only the newest three version folders are kept. To roll back, point `latest.json` at an older
kept version (Prism never downgrades on its own, so this only stops further updates to the bad
one). `min_supported` sends anything older to the download page.

**One-time setup:**

1. `cargo xtask update keygen --secret ~/prism-update.key` writes `packaging/update/public.key`
   (commit it) and the secret key outside the repository. Store the secret key's text as the
   `PRISM_UPDATE_SECRET_KEY` secret in the `release` environment, and keep an offline copy:
   losing it means shipping a new public key by hand.
2. Put the feed URL (`https://<site>/stable/latest.json`) in `packaging/update/feed-url.txt` and
   commit it. Builds without the key or the URL simply have updates turned off.
3. Create the site repository, connect it to Vercel (deploys on push), and set
   `vars.UPDATE_SITE_REPO`, `vars.UPDATE_BASE_URL` and `secrets.UPDATE_SITE_TOKEN` (a token that
   can push to it) in the `release` environment.

**Each release:** after publishing the GitHub Release, run *Actions → Publish update* with the
version. It copies the release's update packages into the site, signs the feed, prunes old
versions and pushes; Vercel deploys. Locally the same step is:

```sh
cargo xtask update publish --site ../prism-updates --base-url https://<site> --secret ~/prism-update.key
```

**Without GitHub Actions** (local builds): `packaging/publish-update.sh` does the same as the
workflow from this machine. It clones `../prism-updates`, publishes `dist/release` with
`~/prism-update.key`, force-pushes a single commit (as the `NK-Studio` gh account) and waits for
the live feed. Add `--dry-run` to only print the feed. RustRover has *Prism Publish Update* and
*Prism Publish Update (dry run)* for it.

**Release checklist:** `cargo xtask release start X.Y.Z` walks a release step by step: version,
commit, macOS build, smoke test, Windows build, tag, installers, feed, update check. The steps
and their checks live in `packaging/release/pipeline.json`, and progress in
`log/release/state.json`. `cargo xtask release status` shows the current step, and
`cargo xtask release done` checks it and moves on. `skip` passes an optional step, `back`
reopens the previous one. The `release` Claude skill (`.claude/skills/release/`) drives the same
commands from a conversation ("what's next?" / "done").

**Testing locally:** build a package with a throwaway key and a local feed, serve it, and run
Help › Check for Updates… from the installed copy:

```sh
cargo xtask update keygen --secret /tmp/test.key --public /tmp/test.pub
PRISM_UPDATE_URL=http://127.0.0.1:8765/stable/latest.json PRISM_UPDATE_PUBKEY="$(tail -1 /tmp/test.pub)" \
  DIST=/tmp/prism-dist packaging/macos/package.sh --arch aarch64
cargo xtask update publish --site /tmp/site --base-url http://127.0.0.1:8765 --dist /tmp/prism-dist --version 9.9.9 --secret /tmp/test.key
python3 -m http.server 8765 --directory /tmp/site
```

The publish step looks for `prism-9.9.9-*` files, so first copy the package under that name.
Debug builds also accept `PRISM_UPDATE_PUBKEY` at run time. `PRISM_UPDATE_URL` overrides the
feed URL at run time in any build, which is harmless because the signature is still checked.

## Secrets

All secrets live in the repository's **`release` environment** (*Settings → Environments →
release*). Restrict its deployment branches to `release` and `main`. Every job in `release.yml` that
signs or publishes declares `environment: release` (the FreeBSD job signs nothing
and does not declare it), and so does the macOS job of `build.yml` (signed Apple Silicon builds of
every `main` push), so only runs on those two branches can read the secrets. Each secret is optional. If one is missing, that platform's artifacts are unsigned and
the run shows a `::warning::`.

| Secret | Used for |
|---|---|
| `APPLE_CERTIFICATE` | base64 `.p12` with "Developer ID Application: <your company> (<team id>)" |
| `APPLE_CERTIFICATE_PASSWORD` | password for that `.p12` |
| `KEYCHAIN_PASSWORD` | password for the temporary CI keychain (random if unset) |
| `APPLE_API_KEY` | App Store Connect API key (`AuthKey_<key id>.p8` contents) used by `notarytool`; preferred over `APPLE_ID` |
| `APPLE_API_KEY_ID` | that key's ID |
| `APPLE_API_ISSUER` | issuer ID (App Store Connect → Users and Access → Integrations) |
| `APPLE_ID` | Apple ID used by `notarytool` when no API key is set |
| `APPLE_PASSWORD` | app-specific password for that Apple ID |
| `APPLE_TEAM_ID` | `9PA2HRW995`, the team ID in the certificate's name; `verify.sh` checks the signature against it |
| `WINDOWS_CERTIFICATE` | base64 `.pfx` code-signing certificate |
| `WINDOWS_CERTIFICATE_PASSWORD` | password for that `.pfx` |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` | service principal for Azure Trusted Signing (an alternative to the `.pfx`) |
| `AZURE_SIGNING_ENDPOINT` | e.g. `https://eus.codesigning.azure.net` |
| `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` | Trusted Signing account and certificate profile names |

`GITHUB_TOKEN` creates the release. Only the final job gets `contents: write`.

On macOS, `packaging/macos/import-cert.sh` decodes the `.p12` into a temporary keychain
(`security create-keychain`, then `security import`, then `set-key-partition-list` so codesign
can use the key without a prompt). It exports the identity's SHA-1 as `MACOS_SIGN_IDENTITY`.
The keychain is deleted at the end of the job.

## Icons

`assets/app-icon/prism.svg` is the canonical icon: the owner's ArtCraft drawing of a
nine-tailed kitsune, vectorised (MIT OR Apache-2.0, see `LICENSE.txt` there; palette and
geometry in `README.md`). `packaging/icons.sh` regenerates the 1024 px PNG, the `.icns`, the `.ico` (packed by
`cargo xtask ico`) and the hicolor PNGs from it. It needs `resvg`, plus `iconutil` on macOS.
The outputs are committed, so packaging never needs those tools.

## Checks

`.github/workflows/packaging-lint.yml` runs in seconds on any change to `packaging/`, the
workflows or the icons. It runs actionlint, shellcheck, a PowerShell parse, xmllint,
`desktop-file-validate` and `appstreamcli validate` for FreeBSD metadata.
