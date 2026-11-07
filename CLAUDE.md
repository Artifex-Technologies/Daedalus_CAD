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
  `portal.rs` the desktop's own open/save dialog (XDG FileChooser portal: Minos on Yggdrasil; the built-in `filedlg.rs` when
  there is no portal); `icons.rs` the toolbar's vector icons (thin line art in the theme's text colour, a few orange accents; no image files; our own drawings: IngeTrazo's look was the reference, from a screenshot only); `settings.rs` units and toolbar side;
  `text3d.rs` 3D Text outlines; `frame.rs` the window's own frame on Linux (resize edges; the menu bar is the title bar, see below); `theme.rs` light or
  dark chrome from the desktop (XDG portal over zbus on Linux, winit on Windows)

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
- 144 tests pass (2026-11-06). The viewer runs on Linux (CachyOS, KDE Wayland, NVIDIA) with egui toolbar, drawing
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
- SketchUp tool set, phase 1 (2026-11-06): the toolbar down the left is the Large Tool Set in SketchUp's order (two
  columns; tools not built yet greyed out in their place), then views and face styles; left is the default. Added:
  Rotated Rectangle, Pie, Move (M, axis lock incl. blue), Rotate (Q, 15 degree snaps, typed degrees), Scale (S, uniform
  from the opposite bounding-box corner, typed factor or length), Tape Measure (T) and Protractor (guides, snapped to;
  Edit > Delete Guides; not saved in the file yet), Zoom Window, Previous view, Front/Back/Left/Right, Position Camera,
  Look Around, Walk (arrow keys too), face styles Wireframe / Hidden Line / Shaded / X-Ray and Back Edges (remembered).
  Selection is a set: Ctrl adds, Shift toggles, triple-click selects everything joined, Ctrl+A, Delete erases.
  Move/Rotate/Scale move vertices (`Model::transform_vertices`): a lone face moved sideways stretches its neighbours.
- Light/dark (2026-11-06): the chrome follows the desktop live; the modelling area stays white until a background and
  lighting are chosen. In dark mode each toolbar icon sits on a light tile (the icons are dark ink).
- Lines (2026-11-06, phase 2 started): `Mesh.lines` / `HalfEdgeMesh.lines` are loose edges. The Line tool puts each
  segment in the model at once (one undo step each; Esc keeps what is drawn), welding onto existing vertices; a closed loop
  of loose lines in one plane becomes a face (`close_loop`, shortest path). Erasing a face leaves its edges as lines; the
  Eraser takes a line under the mouse first. Line works in 3D: snap > red/green/blue axis through the last point (ray
  lock, so blue works) > face/ground. Snaps include the origin and points on the axes. `.daedalus` version 2 holds `l a b`
  (a file without lines is still written as version 1). Not yet: lines split faces (imprint), lines crossing (weld).
- Phase 2 done, phase 3 started (2026-11-06, `core/src/imprint.rs`): a line joins corners and edges it lands on, breaks
  at every edge it crosses, and a piece across a face splits it; loops close against face edges too; an outline wholly
  inside a face becomes a face with a hole cut round it (faces have one outline, so the ring is two faces joined by two
  hidden seams, `Mesh::seams`, saved as `s a b`). Shape tools draw through this (`Model::add_outline`). New tools: Arc,
  2 Point Arc (A), 3 Point Arc, Freehand, Offset (F, inward only; Repair moved to the Tools menu), Follow Me (open and
  closed paths, mitred), Paint Bucket (B; `Mesh::colors`, saved as `c face r g b`; Alt+click samples), Monochrome style,
  Section Plane (shader clip, not saved), Dimensions (not saved).
- Phase 3 done (2026-11-06, `core/src/scene.rs`): groups and components. A `Scene` is the top `Context` (mesh plus
  instances), `Definition`s (each a context) and `Notes` (guides, dimensions, text, section, axes). `Model` holds the scene and
  the geometry being edited (`mesh()`, in model coordinates even inside a group: `open`/`close` swap it in and out), with
  whole-scene undo snapshots. Make Group / Make Component (G), double-click to edit (rest faded), Explode, Make Unique,
  nesting; Move/Rotate/Scale take instances (`Model::transform_all`), Ctrl during Move copies, `x5` / `/5` arrays
  (`copy_all`). Text tool (leader, default area/position/name, retype by double-click), Axes tool (ground and axis locks
  follow them; Reset Axes), 3D Text (`viewer/src/text3d.rs`: Ubuntu Light from `epaint_default_fonts`, ttf-parser; letters
  to faces with holes and extruded in `core/src/profile.rs`; placed as a group), section fill (back faces through the cut,
  shader) and cut lines (`core/src/section.rs`). `.daedalus` version 3: `def`/`end`, `i`, `guide`, `dim`, `text`,
  `section`, `axes`. Shaders are validated in a test (naga). Not seen on screen yet.
- Exit (2026-11-06): the window and GPU state are dropped before the event loop ends, and wgpu starts Vulkan only (OpenGL
  just when Vulkan finds no adapter): NVIDIA's EGL clean-up after the Wayland connection closed crashed at every close
  (SIGSEGV, Sarina reported it as a crash).
- SketchUp (2026-11-06): `.skp` opens through `tools/skp-convert` (Python, OpenSKP, MIT; installed in its own venv in
  ~/.local/share/daedalus/openskp, not part of the build), which writes `.dskp` (our text format) that
  `daedalus-io/src/skp.rs` builds into groups and components; faces that would make an edge non-manifold or clash in
  winding go into a nested "(overlapping faces)" group. Matt's 29 models converted (OBJ checked: sizes right); the .dskp
  path is unit-tested, not yet tried on his files. Claude Code's safety check does not let Claude run OpenSKP itself.
- Edges in the selection (2026-11-06): a Select click within 6 px of an edge or loose line of the geometry being edited takes
  the line (blue), not the face behind it (Ctrl adds, Shift toggles; a triple-click still takes everything joined). Delete
  (`HalfEdgeMesh::erase_edges`): a loose line goes; an edge between two faces in one plane joins them (`merge::join_pair`;
  kept, with a message, if they would not make one simple face); any other edge takes the faces on it, and faces seamed to
  them. Move/Rotate/Scale carry selected edges' vertices.
- Precision (2026-11-06): positions, tools, camera and formats are f64 (the GPU gets f32 when drawing; binary STL stays
  32-bit). Tolerances follow the model: `Mesh::tolerance()` = size x 1e-7 (lines, coplanar merge, welds; Repair x10).
- Faces with holes (2026-11-06): `Mesh::holes` (per face, wound against the outline; `holes_of`, `loops`, `face_edges`),
  `HalfEdgeMesh` keeps hole loops per face. A shape drawn inside a face cuts a true hole (`imprint::cut_hole`); Push/Pull
  builds walls round each hole; splits, merges, Erase, Repair, section cuts and 3D Text carry holes; triangles by bridging
  (`triangulate_with_holes`). `.daedalus` 4 writes `h` lines; older files' two-face rings (seams) are joined back
  (`Mesh::join_seamed_faces`). SketchUp faces come in with their holes (pinched outlines allowed). OBJ/DAE export a face
  with holes as triangles (`Mesh::polygons`). Not yet: Follow Me sweeps only a profile's outline.
- Edges are drawn as screen-facing strips 1.4 px wide (lines.wgsl, one instance per segment) with 4x MSAA, pulled a
  hair toward the eye so faces never break them up.
- The SketchUp tool phases still to come are in PROGRESS.md ("SketchUp tool set").
- Next milestones: Solid Tools (phase 4), Sandbox (phase 5), constraints, sheet metal inside the model, STEP export.
- Biggest open decisions: geometry kernel (half-edge mesh vs OpenCASCADE bindings vs pure-Rust B-rep),
  constraint solver, reuse CADCraft crates or reimplement.

## Fabrication facts (SendCutSend, public guidelines)
Flat DXF: 1:1, inches preferred (mm fine if consistent), all cut geometry on one layer, closed paths, no
duplicate lines or stray points, text outlined, a line at the centre of each bend. Bent parts need 3D STEP
using the shop's own K-factor and radius; square bend reliefs. Unverified: whether a separate `BEND` layer
is accepted next to the cut layer.
