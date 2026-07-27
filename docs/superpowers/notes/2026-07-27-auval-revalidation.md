# auval re-validation — PR #11 review fixes (run after a reboot)

**Why:** the PR #11 review-fix build needs a fresh-bundle `auval` run. The
in-session attempt was blocked: rebuilding over the launchd-registered AUv3
appex tombstoned the `aumi DrmS SFor` component, and `lsregister` / `pluginkit`
/ `open` / `killall AudioComponentRegistrar` did **not** recover it. A reboot
clears the tombstone; this file is the deterministic re-validation procedure.

**Expected result:** `auval -v aumi DrmS SFor` → `AU VALIDATION SUCCEEDED.`,
this time against the review-fix build at `/Applications/StepForge-macOS.app`.

---

## A. Quick path (the fresh Debug build is already installed)

`/Applications/StepForge-macOS.app` already holds the review-fix build (Debug).
If you don't need to rebuild:

```bash
# 1. Reboot to clear the launchd AU-registration tombstone.
sudo reboot
#    …log back in…

# 2. Register the embedded appex by launching the host app once.
open /Applications/StepForge-macOS.app

# 3. Confirm the component is registered again (expect one line).
auval -l | grep -i 'aumi.*drms.*sfor'

# 4. Validate.
auval -v aumi DrmS SFor        # expect: AU VALIDATION SUCCEEDED.
```

If step 3 lists nothing, see *Troubleshooting* below.

## B. Full path (rebuild from the review-fix branch, e.g. Release)

Use this to validate a clean Release build, or if `/Applications` no longer
holds the review-fix code. **Do not** let `xcodebuild` write to the registered
bundle path — that is exactly what tombstoned registration last time. Build
into a *separate* derived-data dir, then copy the finished app into place.

```bash
export PATH="$HOME/.cargo/bin:$PATH"      # rustup shadow (Homebrew rust can't cross-build)
cd <repo>/app
xcodegen generate                          # regenerate StepForge.xcodeproj

# Build to a SEPARATE derived data (not the default DerivedData, which is
# where the component may be registered).
xcodebuild -project StepForge.xcodeproj -scheme StepForge-macOS \
  -configuration Release -destination 'generic/platform=macOS' \
  -derivedDataPath ./build CODE_SIGNING_ALLOWED=NO build

# Install the finished app to the stable registered path, then re-register.
rm -rf /Applications/StepForge-macOS.app
cp -R ./build/Build/Products/Release/StepForge-macOS.app /Applications/
open /Applications/StepForge-macOS.app

auval -l | grep -i 'aumi.*drms.*sfor'      # confirm registered
auval -v aumi DrmS SFor                    # expect: AU VALIDATION SUCCEEDED.
```

---

## Troubleshooting

- **`auval -l` lists nothing for StepForge after `open`:** reboot first if you
  haven't (the tombstone survives `killall`). Then re-run `open` and retry.
- **Duplicate/conflicting registrations** (same `aumi/DrmS/SFor` from multiple
  paths → the registrar may register none): remove stray copies, leaving only
  one:
  ```bash
  rm -rf ~/Library/Audio/Plug-Ins/Components/StepForgeAU.appex
  rm -rf ~/Library/Developer/Xcode/DerivedData/StepForge-*/Build/Products/Debug/StepForge-macOS.app
  killall AudioComponentRegistrar 2>/dev/null
  open /Applications/StepForge-macOS.app
  ```
- **`auval` still can't find it after a reboot + `open`:** confirm the appex
  exists and loads —
  `ls /Applications/StepForge-macOS.app/Contents/PlugIns/StepForgeAU.appex` —
  and that the host app actually stays running (`pgrep -fl StepForge-macOS`).
  An appex that won't register after a clean reboot points at a real
  codesign/load problem in the build, not the cache.

## Robust pattern for future AU builds

To avoid re-tombstoning: keep **one** stable install location
(`/Applications/StepForge-macOS.app`), always build into a *separate*
`-derivedDataPath`, then `cp -R` the finished app over the install path and
`open` it to re-register. Never point `xcodebuild` (or `build_install_macos.sh`)
at the path a component is currently registered at mid-session.
