# Daedalus CAD

## What this is

Multi-purpose CAD in Rust: SketchUp-style 3D modeling for buildings and layouts, top-down 2D drafting, and mechanical part
design including sheet metal (flat patterns and bend lines for fabrication services such as SendCutSend) and stress testing
(linear static FEA). A Cargo workspace under `crates/` (layout below). Primary target is **Arch Linux**; Windows must keep
building too.

**UX rule: match SketchUp's layout, tools and controls.** Other CAD programs are references for extra capability only.

## Build and test

Cap every build and test:

```sh
CAP="systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0 env CARGO_BUILD_JOBS=4 nice -n 19"
$CAP cargo test --workspace                      # no GPU needed; the main check
$CAP cargo build --workspace
cargo run -p daedalus-viewer [-- model.stl]      # the modeller (wgpu + egui)
cargo run -p daedalus-app -- inspect model.stl [--fix out.stl]
cargo run -p daedalus-app -- flatten out.dxf --thickness 2 --k 0.44 --width 40 50 90:2 30
cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu   # from Windows
```

`daedalus-sheet` and its dependencies are optimised in debug builds (root `Cargo.toml`), or its tests take minutes. The tool
cursors' contact sheet: `CURSOR_SHEET=out.png cargo test sheet -- --ignored`. Arch setup (packages, Vulkan driver) is in
`README.md`. CI (`.github/workflows/ci.yml`, every push): in an Arch container `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets --locked -- -D warnings`, build and test; on Windows build and test. After a push, watch the run for
the full commit: `gh run list --commit <sha>`.

## Rules

- Privacy: no personal details in commits, code or docs: no names of people, host names, network addresses, SSIDs or MAC
  addresses. Scan the staged diff before every push. Write docs in the first person as the owner.
- Push every commit as you make it; stage files by explicit path; `git pull --rebase` first.
- **Clean-room policy (important).** Other modelers are studied through public documentation and by using them. **Do not decompile applications,
and do not read or port source from copyleft projects** (IngeTrazo GPL-3, LibreCAD GPL-2, FreeCAD LGPL-2.1,
LibreDWG GPLv3). Behaviour is reimplemented independently so the Daedalus licence stays free to choose
(currently `MPL-2.0`, see `LICENSE`). CADCraft (MIT/Apache) is the one permissive reference; depending on its crates
is an open decision, not yet taken.
- Every kernel or format feature gets a unit test (more under Know-how).
- Never run OpenSKP from a session: `tools/skp-convert` is run by me.

## Contracts

- What Daedalus provides (the build Labyrinth OS packages, the `.ddls` file, `.dskp`) and what it uses (the desktop's XDG
  portals, SolidCraft's kernel): [docs/CONTRACTS.md](docs/CONTRACTS.md).

## Open work

docs/plans/BACKLOG.md and docs/plans/LOG.md (private; ask the owner).

## Know-how

### Layout
- `crates/daedalus-core`: `Mesh` (interchange, `pick_face`, `edges`), `HalfEdgeMesh` (topology, `push_pull`,
  `split_face`, `erase_face`), `Model` (document + undo/redo), `merge` (coplanar faces), `sketch` (workplanes and shapes), `triangulate`
  (ear clipping), `inspect` (solid checks and repair), `sheetmetal` (flat patterns)
- `crates/daedalus-io`: `MeshImporter` / `MeshExporter` per format (`.ddls` native, earlier `.daedalus`; STEP in `step.rs`,
  our own AP214 reader and writer: flat faces, assemblies, colours, units, loose lines as wireframe, soft edges in an
  invisible "Soft edges" layer; STL, OBJ, DXF, DAE), `formats.rs` (where Save writes, what each format leaves out),
  `dxf2d` (flat-pattern DXF), images/PSD
- `crates/daedalus-app`: CLI (`inspect`, `flatten`, stats)
- `crates/daedalus-sheet`: exact solids (B-rep) on SolidCraft's kernel (MIT/Apache, git dependency pinned to commit
  871014f; solidcore underneath is Apache-2.0; notices in the crate's `NOTICE.md`). `sheet.rs` is SolidCraft's sheet-metal
  model copied and cut loose from its document (rules are plain numbers, `RuleValues`); `util.rs` the error type and 4x4
  helpers it needs. Rust 2024 edition for this crate only. Dependencies and this crate are optimised in debug builds
  (root `Cargo.toml`), or its tests take minutes. STEP comes from `kernel::step_export` (AP242, true cylinders).
- `crates/daedalus-fea`: stress testing (linear static FEA). `voxel` fills a closed part with cubes, `hex8` the cube element
  (8-node brick with Wilson's incompatible modes, condensed once; corner strains with the bubbles in), `solve` K u = f by
  Jacobi-preconditioned conjugate gradients without storing K, `material` typical presets; `run` matches supports and loads to
  model faces and reports peak von Mises, safety factor, deflection, reactions and the coloured surface. No dependencies.
- `crates/daedalus-viewer`: wgpu 24 + winit 0.30 + egui 0.31 (pinned together on purpose; egui 0.32 needs wgpu 25).
  `tools.rs` is the GPU-free tool state machine (unit-tested); `render.rs` GPU; `ui.rs` egui chrome; `main.rs` events,
  snapping and the document (open, save, unsaved changes); `filedlg.rs` the built-in file browser (no GTK/Qt dialog);
  `portal.rs` the desktop's own open/save dialog (XDG FileChooser portal: Minos on Yggdrasil; the built-in `filedlg.rs` when
  there is no portal); `icons.rs` the toolbar's vector icons (thin line art in the theme's text colour, a few orange accents; no image files; our own drawings: IngeTrazo's look was the reference, from a screenshot only); `settings.rs` units and toolbar side;
  `text3d.rs` 3D Text outlines; `frame.rs` the window's own frame on Linux (resize edges; the menu bar is the title bar, see below); `theme.rs` light or
  dark chrome from the desktop (XDG portal over zbus on Linux, winit on Windows)

### Conventions
- Every kernel or format feature gets a unit test. Solids are verified with **signed volume and Euler
  characteristic**; flat patterns are cross-checked against bend deduction by hand-computed numbers.
- Edits are atomic undo steps; a failed edit must leave the model unchanged (`Model::edit`).
- Face indices are **not stable** across push/pull or repair (a known limitation).
- Importers/exporters stay behind the traits. Formats with licence or SDK constraints (DWG, SKP, FBX) go in
  optional, feature-gated modules.
- Keep K-factor, bend radius and other fabricator values as inputs, never constants.

### File formats
- Save keeps the format a model was opened in when Daedalus writes it (STEP, `.ddls`, OBJ, STL, DAE, DXF), in place with no
  dialog (`formats::save_target`); new models default to STEP. `.skp`/`.dskp`, and STEP files read only in part, are saved
  beside themselves as STEP. A lossy save names what was left out and offers a full `.ddls` copy.
- `.ddls` is versioned: a model with soft edges must be written as version 4 or later (versions 2 and 3 join faces across
  seams on reading).
- File > Export STEP for Fabrication is the exact exporter in `daedalus-sheet` (AP242); `daedalus-io/src/step.rs` is our own
  AP214 reader and writer (flat faces).

### The window on Linux
No system frame (Hyprland draws none): the menu bar is the title bar (drag to move, double-click to maximize, buttons at its
right end), the outer 6 px resize. Hyprland reports a floating window as maximized, so "maximized" also needs the window to
fill its screen (`frame::fills`). wgpu starts Vulkan only (OpenGL just when Vulkan finds no adapter): NVIDIA's EGL clean-up
after the Wayland connection closed crashed at every close; the window and GPU state are dropped before the event loop ends.

### Fabrication facts (SendCutSend, public guidelines)
Flat DXF: 1:1, inches preferred (mm fine if consistent), all cut geometry on one layer, closed paths, no
duplicate lines or stray points, text outlined, a line at the centre of each bend. Bent parts need 3D STEP
using the shop's own K-factor and radius; square bend reliefs. Unverified: whether a separate `BEND` layer
is accepted next to the cut layer.
