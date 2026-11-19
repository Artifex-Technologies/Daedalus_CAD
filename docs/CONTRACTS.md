# Contracts

What other projects may rely on from Daedalus CAD, and what it uses from them. One section per contract.

## Provided by Daedalus CAD

### The build for packages

`cargo build --release --locked -p daedalus-viewer -p daedalus-app` gives the modeller (`daedalus-viewer`, installed as
`daedalus`) and the command line (`daedalus-app`, installed as `daedalus-cli`); `cargo test --release --locked --workspace`
must pass without a GPU. `packaging/` holds the desktop entry (`daedalus.desktop`), the icon (`daedalus.svg`) and the file
types (`daedalus-mime.xml`). `VERSION` is Daedalus's own version; the package version is `VERSION-<build number>`.
Used by: Labyrinth OS's `daedalus-cad-git` recipe (and through it Agora's catalog).

### The native file (`.ddls`)

Plain text, versioned (version 6 holds sheet-metal parts), keeps everything; `.daedalus`, its earlier name, still opens.
A newer Daedalus reads every older version. Written and read by `crates/daedalus-io/src/native.rs`.
Used by: Daedalus itself; nothing else reads it yet.

### `.dskp` (from `tools/skp-convert`)

The text form `tools/skp-convert` writes from a SketchUp `.skp` (through OpenSKP, installed apart) and
`crates/daedalus-io/src/skp.rs` reads. Both sides are in this repository; the converter is not part of the build.

## Used by Daedalus CAD (provided elsewhere)

| Contract | Provider | Used here by |
| --- | --- | --- |
| XDG FileChooser portal (open and save dialogs) | the desktop's portal (on Yggdrasil: `yggdrasil-portal`, Minos) | `crates/daedalus-viewer/src/portal.rs`; the built-in `filedlg.rs` when there is no portal |
| XDG Settings portal (`color-scheme`) | the desktop's portal | `crates/daedalus-viewer/src/theme.rs` (light or dark chrome) |
| SolidCraft's kernel (git dependency pinned to commit 871014f) | SolidCraft (MIT/Apache) | `crates/daedalus-sheet` |
