# photokit-probe

A read-only test of reading Apple Photos through PhotoKit from pure Rust (`objc2-photos`), for
[storytold/lightcraft#1622](https://github.com/storytold/lightcraft/issues/1622). Not part of LightCraft.

It answers two questions:

1. **Does PhotoKit from Rust need `unsafe`?** Every PhotoKit call is in its own `unsafe` block with
   `unused_unsafe` warnings on; `unsafe-report.txt` (from the build) lists the calls Rust treats as safe.
2. **Who gets Photos access?** Three variants of the same probe:
   - `photokit-probe`: a plain command-line tool;
   - `photokit-probe-plist`: the same with an Info.plist (usage description) embedded in the binary;
   - `PhotoKitProbe.app`: an ad-hoc-signed app bundle.

It never changes the library. It prints album names and counts and, for the first few items, their
identifier, type, size, date, favourite flag and original file names. **Run it against a test library**
(hold Option while opening Photos, choose the test library, keep Photos open).

## Run

```sh
gh run download <run-id> -R GLYSK-OU/photokit-probe -D ~/Downloads/pkp
cd ~/Downloads/pkp/photokit-probe-*/dist
xattr -dr com.apple.quarantine .
./photokit-probe list 5          # plain tool: what macOS says
./photokit-probe-plist list 5    # with embedded Info.plist
open PhotoKitProbe.app; sleep 5; cat /tmp/photokit-probe-app.txt
```

Commands: `status` (no prompt), `request` (ask for access), `list [N]` (ask if needed, then list; default).
Each run also writes its report to `/tmp/photokit-probe-<variant>.txt`.
