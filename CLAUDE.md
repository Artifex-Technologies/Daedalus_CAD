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
cargo run -p daedalus-viewer [-- model.stl] # wgpu viewport
cargo run -p daedalus-app -- inspect model.stl [--fix out.stl]
cargo run -p daedalus-app -- flatten out.dxf --thickness 2 --k 0.44 --width 40 50 90:2 30
cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu   # from Windows
```
Arch setup (packages, Vulkan driver) is in `README.md`. CI: `.github/workflows/ci.yml` (Arch container + Windows, `--locked`).

## Layout
- `crates/daedalus-core`: `Mesh` (interchange), `HalfEdgeMesh` (topology, `push_pull`, `split_face`, `pick_face`),
  `Model` (document + undo/redo), `inspect` (solid checks and repair), `sheetmetal` (flat patterns)
- `crates/daedalus-io`: `MeshImporter` / `MeshExporter` per format (STL, OBJ, DXF, DAE), `dxf2d` (flat-pattern DXF), images/PSD
- `crates/daedalus-app`: CLI (`inspect`, `flatten`, stats)
- `crates/daedalus-viewer`: wgpu 24 + winit 0.30 viewport (pinned on purpose; newer majors changed APIs)

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
(currently `MIT OR Apache-2.0`). CADCraft (MIT/Apache) is the one permissive reference; depending on its crates
is an open decision, not yet taken.

## Where the plan lives
`PROGRESS.md` (gitignored, local) holds the roadmap, done list, open questions and the Linux first-run checklist.
If it is missing on a fresh clone, it was not copied over: ask the owner. The essentials are repeated here;
the current state at the time of the Windows to Linux move is:
- 39 tests pass; viewer launched on Windows; **never built or run on Linux**; first job on Arch is the
  first-run checklist (tests, viewer opens, orbit/pan/zoom, click-select, Up/Down push/pull, Ctrl+Z/Y,
  Wayland and X11, `WGPU_BACKEND=gl` if no Vulkan driver).
- Next milestones: drawing tools (line/rectangle/circle/polygon/arc), plan view that toggles to 3D,
  inference and snapping, exact value entry, constraints, groups/components, native `.daedalus` format,
  sheet metal inside the model, STEP export for bent parts.
- Biggest open decisions: geometry kernel (half-edge mesh vs OpenCASCADE bindings vs pure-Rust B-rep),
  UI toolkit, constraint solver, reuse CADCraft crates or reimplement.

## Fabrication facts (SendCutSend, public guidelines)
Flat DXF: 1:1, inches preferred (mm fine if consistent), all cut geometry on one layer, closed paths, no
duplicate lines or stray points, text outlined, a line at the centre of each bend. Bent parts need 3D STEP
using the shop's own K-factor and radius; square bend reliefs. Unverified: whether a separate `BEND` layer
is accepted next to the cut layer.
