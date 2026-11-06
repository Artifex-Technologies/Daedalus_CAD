# Daedalus CAD

Multi-purpose CAD in Rust: SketchUp-style 3D modeling for buildings and layouts, top-down 2D drafting,
and mechanical part design including sheet metal (flat patterns and bend lines for fabrication services
such as SendCutSend). Primary target is **Arch Linux**; Windows must keep building too.

**UX rule: match SketchUp's layout, tools and controls.** Other CAD programs are references for extra
capability only.

## Commands
```sh
cargo test --workspace                      # no GPU needed; the main check
cargo build --workspace
cargo run -p daedalus-viewer [-- model.stl] # the modeler (wgpu + egui)
cargo run -p daedalus-app -- inspect model.stl [--fix out.stl]
cargo run -p daedalus-app -- flatten out.dxf --thickness 2 --k 0.44 --width 40 50 90:2 30
cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu   # from Windows
```
Arch setup (packages, Vulkan driver) is in `README.md`. CI: `.github/workflows/ci.yml` (Arch container + Windows, `--locked`).

## Layout
- `crates/daedalus-core`: `Mesh` (interchange, `pick_face`, `edges`), `HalfEdgeMesh` (topology, `push_pull`,
  `split_face`, `erase_face`), `Model` (document + undo/redo), `merge` (coplanar faces), `sketch` (workplanes and shapes), `triangulate`
  (ear clipping), `inspect` (solid checks and repair), `sheetmetal` (flat patterns)
- `crates/daedalus-io`: `MeshImporter` / `MeshExporter` per format (`.daedalus` native, STL, OBJ, DXF, DAE), `dxf2d`
  (flat-pattern DXF), images/PSD
- `crates/daedalus-app`: CLI (`inspect`, `flatten`, stats)
- `crates/daedalus-viewer`: wgpu 24 + winit 0.30 + egui 0.31 (pinned together on purpose; egui 0.32 needs wgpu 25).
  `tools.rs` is the GPU-free tool state machine (unit-tested); `render.rs` GPU; `ui.rs` egui chrome; `main.rs` events,
  snapping and the document (open, save, unsaved changes); `filedlg.rs` the built-in file browser (no GTK/Qt dialog);
  `icons.rs` the toolbar's vector icons (SketchUp-style pictures, no image files); `settings.rs` units and toolbar side;
  `frame.rs` the window's own frame on Linux (resize edges; the menu bar is the title bar, see below)

## Conventions
- Every kernel or format feature gets a unit test. Solids are verified with **signed volume and Euler
  characteristic**; flat patterns are cross-checked against bend deduction by hand-computed numbers.
- Edits are atomic undo steps; a failed edit must leave the model unchanged (`Model::edit`).
- Face indices are **not stable** across push/pull or repair (known limitation, see roadmap).
- Importers/exporters stay behind the traits. Formats with licence or SDK constraints (DWG, SKP, FBX) go in
  optional, feature-gated modules.
- Keep K-factor, bend radius and other fabricator values as inputs, never constants.

## Clean-room policy (important)
Other modelers are studied through public documentation and by using them. **Do not decompile applications,
and do not read or port source from copyleft projects** (IngeTrazo GPL-3, LibreCAD GPL-2, FreeCAD LGPL-2.1,
LibreDWG GPLv3). Behaviour is reimplemented independently so the Daedalus licence stays free to choose
(currently `MPL-2.0`, see `LICENSE`). CADCraft (MIT/Apache) is the one permissive reference; depending on its crates
is an open decision, not yet taken.

## Where the plan lives
`PROGRESS.md` (gitignored, local) holds the roadmap, done list, open questions and the Linux first-run checklist.
If it is missing on a fresh clone, it was not copied over: ask the owner. The essentials are repeated here;
the current state is:
- 80 tests pass (2026-11-06). The viewer runs on Linux (CachyOS, KDE Wayland, NVIDIA) with egui toolbar, drawing
  tools (Line, Rectangle, Circle, Polygon), push/pull by mouse or typed value, Eraser, endpoint/midpoint snapping
  and axis lock, Measurements box, edges, Top/Iso and parallel projection. UI toolkit decided: egui.
- Driven by hand 2026-11-05 (Hyprland, Wayland and XWayland): Rectangle with typed size, Push/Pull with typed distance,
  undo/redo, orbit/zoom/pan, Solid Inspector and repair on an open STL, File > Save/Open/New and the unsaved-changes question.
  Not yet: `WGPU_BACKEND=gl`, HiDPI scaling.
- Known gaps: shapes drawn on a face overlap it instead of splitting it (no imprinting, no faces with holes);
  touching shapes do not share edges (no vertex welding); no arcs.
- Toolbar (2026-11-05): icons instead of words, the name and shortcut on hover; View > Toolbar puts it across the top or
  down the left side (remembered).
- Units (`core/src/units.rs`, 2026-11-05): the model is in metres; Units menu shows and types lengths in mm, cm, m, inches
  or feet and inches (to 1/16"), remembered in `~/.config/daedalus/settings.conf`. A written unit always wins when typing
  (`300mm`, `4' 6 1/2"`); once a number is being typed, unit letters, `'`, `"`, `/` and spaces go to the Measurements box.
- Coplanar faces merge (`core/src/merge.rs`, 2026-11-05): after every push/pull (one undo step), in Repair, and by Tools >
  Merge Coplanar Faces. A group whose outline would have a hole or a pinch is left alone.
- Native format: `.daedalus` (text, versioned, `crates/daedalus-io/src/native.rs`); File menu with New/Open/Save/Save As/
  Import/Export, a dot in the title while unsaved, and a question before unsaved changes are thrown away (2026-11-05).
- Window frame (2026-11-06): on Linux the window has no system frame (Hyprland draws none, so it could not be moved or
  resized with the mouse). The menu bar is the title bar: the title in its empty part, drag to move, double-click to
  maximize, round minimize/maximize/close buttons at its right end (as on the Yggdrasil desktop); the outer 6 px resize.
  Hyprland reports a floating window as maximized, so "maximized" also needs the window to fill its screen
  (`frame::fills`). Opens at 1280x800. Windows keeps its system frame. Driven by hand on Hyprland 0.57: move, resize,
  maximize/restore, minimize, close.
- Next milestones: imprint and weld, arcs, Move/Rotate/Offset, groups/components,
  constraints, sheet metal inside the model, STEP export for bent parts.
- Biggest open decisions: geometry kernel (half-edge mesh vs OpenCASCADE bindings vs pure-Rust B-rep),
  constraint solver, reuse CADCraft crates or reimplement.

## Fabrication facts (SendCutSend, public guidelines)
Flat DXF: 1:1, inches preferred (mm fine if consistent), all cut geometry on one layer, closed paths, no
duplicate lines or stray points, text outlined, a line at the centre of each bend. Bent parts need 3D STEP
using the shop's own K-factor and radius; square bend reliefs. Unverified: whether a separate `BEND` layer
is accepted next to the cut layer.
