# Daedalus CAD

Multi-purpose CAD in Rust: SketchUp-style 3D modeling for buildings and layouts, top-down 2D drafting,
and mechanical part design including sheet metal (flat patterns and bend lines for fabrication services
such as SendCutSend). Primary target is **Arch Linux**; Windows must keep building too.

**UX rule: match SketchUp's layout, tools and controls.** Other CAD programs are references for extra
capability only.

Push every commit as you make it; don't ask me first. Before pushing, check the diff names no host name,
network address, SSID or MAC address.

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
- 331 tests pass (2026-11-07, 120 of them in daedalus-sheet, 10 in daedalus-fea). The viewer runs on Linux (CachyOS, KDE Wayland, NVIDIA) with egui toolbar, drawing
  tools (Line, Rectangle, Circle, Polygon), push/pull by mouse or typed value, Eraser, endpoint/midpoint snapping
  and axis lock, Measurements box, edges, Top/Iso and parallel projection. UI toolkit decided: egui.
- Driven by hand 2026-11-05 (Hyprland, Wayland and XWayland): Rectangle with typed size, Push/Pull with typed distance,
  undo/redo, orbit/zoom/pan, Solid Inspector and repair on an open STL, File > Save/Open/New and the unsaved-changes question.
  Not yet: `WGPU_BACKEND=gl`, HiDPI scaling.
- The 2026-11-05 gaps are closed: imprinting, vertex welding and arcs (Phase 2/3 below), faces with holes and curves
  (2026-11-06 below).
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
  winding go into a nested "(overlapping faces)" group. My 29 models converted (OBJ checked: sizes right); the .dskp
  path is unit-tested, and since 2026-11-07 tried on my files: all 29 open (`cargo run -p daedalus-io --example
  open_check -- FILE...`), the largest about 51,000 faces, and each saves to `.daedalus` and reopens the same. That check found
  a bug, fixed: a model with soft edges was saved as version 2 or 3, whose reader joins faces across seams, so three parts lost
  about 40 % of their faces on the way back; such a model is now written as version 4. Claude Code's safety check does not let Claude run OpenSKP itself.
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
- Curves (2026-11-06, `core/src/curve.rs`): arcs and circles stay segments but are recognised from their shape (3+ equal
  segments in a plane, same centre and radius, same turn of at most 60 degrees: rectangles and hexagons are not curves).
  A Select click on a segment takes the whole curve. Entity Info (right-click menu or View menu; a panel on the right)
  shows a curve's radius, segments (both editable until it is pushed/pulled into walls, as in SketchUp) and length; an
  edge's length; a face's area and holes; a group's name. Right-click menu checked on screen 2026-11-06.
- Loose ends done 2026-11-06: Offset outward (a ring face round the face), Follow Me with holes (tubes), several section
  planes (one cuts; View > Section Planes), Scale grips per axis (corner / edge middle / face middle), Outliner (tray with
  Entity Info), 3D Text in any installed font (`text3d::fonts`), component gluing (`core/src/opening.rs`: Glue to faces
  keeps a component on its face when moved; Cut opening cuts a true hole where its edges touch a face, following it when
  moved and gone when erased; `def ... component glue cut` in the file).
- Textures (2026-11-06): a face's paint is `Paint { rgb, texture }` (`Mesh::colors`), textures live in `Scene::textures`
  (name, image path, tile size in metres; `.daedalus` 4: `texture` lines, a texture number after a `c` line's colour).
  Paint Bucket: "Texture from file…" (Minos), Entity Info sets the tile size. GPU: one RGBA texture array, each image
  resized to 512 px (`render::TEXTURE_SIZE`), repeat sampler; tiles lined up with the face (level across). Style "Shaded
  with Textures" (code 4); Shaded shows a textured face in its average colour. Monochrome drew as Hidden Line before
  (shader); fixed.
- Phase 4, Solid Tools (2026-11-06): `core/src/csg.rs` (BSP booleans over triangulated faces; pieces welded, T-junctions
  split, coplanar pieces merged back; paint kept) and `Model::solid_tool` (Outer Shell, Intersect, Union, Subtract = first
  taken from second and first goes, Trim = same but first stays, Split = three groups). Needs closed groups/components;
  results are new groups, one undo step. Tools: click one solid, then another (toolbar group, Tools > Solid Tools).
  Checked on screen. Outer Shell joins two at a time (SketchUp takes many).
- Phase 5, Sandbox (2026-11-06): `core/src/sandbox.rs` (Delaunay with points nudged a hair for co-circular contours,
  grid, Smoove, Stamp, Drape, Add Detail, Flip Edge) and Model functions. From Contours and Add Detail act on the
  selection; From Scratch (two corners; typed number first = spacing), Smoove (click, then height; typed number first =
  radius; a click on a ground group opens it), Stamp/Drape (object, then ground group), Flip Edge (click an edge). Draw >
  Sandbox and Tools > Sandbox menus, a toolbar group. Contour terrains are all soft inside; grids hide only diagonals.
  Checked on screen (From Scratch, Smoove).
- Tool cursors (2026-11-06, `viewer/src/cursors.rs`): over the model each tool but Select has its own cursor, drawn from
  its toolbar icon (icons.rs records shapes, `icons::shapes`; tessellated by egui and rasterised here): a crosshair at the
  click point with the icon beside it, navigation tools the icon alone; dark ink, white rim. Made at the window's scale
  when it opens. egui is told the arrow while a tool cursor shows, so it takes over again over the chrome. Contact sheet:
  `CURSOR_SHEET=out.png cargo test sheet -- --ignored`. Not yet seen live (nested screenshots leave the cursor out).
- Phase 6, Dynamic Components (2026-11-06, `core/src/dynamic.rs`): attributes on a definition (values or formulas;
  `.daedalus` 5 `attr` lines); built-ins X Y Z LenX LenY LenZ RotZ place and size every copy (`Model::set_attributes`);
  onClick SET/ALTER (Interact tool). Panels in the tray: Component Options (labelled attributes, Enter applies) and
  Component Attributes (rows, + Built-in, Apply). Checked on screen. Since 2026-11-07 also Hidden, Copies (with COPY in
  formulas), Material and `Parent!` references (`model::dc_view`, applied wherever the scene is flattened: drawing, export,
  picking, the stress test). Per-copy values (2026-11-07): Component Options and Interact change the selected copy alone; a
  component placed more than once gets a definition of its own first (`Door#2`, as SketchUp does,
  `Model::set_copy_attributes`), while Component Attributes changes the component and every copy. Location skipped for now
  (my call, 2026-11-06).
- Edges are drawn as screen-facing strips 1.4 px wide (lines.wgsl, one instance per segment) with 4x MSAA, pulled a
  hair toward the eye so faces never break them up.
- The SketchUp tool phases still to come are in PROGRESS.md ("SketchUp tool set").
- Sheet metal and STEP (2026-11-06): `daedalus-sheet` (SolidCraft's sheet model, 99 of its tests) plus `part.rs` (a
  `SheetPart`: material, rule values, base outline, flanges in order; the sheet rebuilt from them, so flanges change or go)
  and `export.rs` (whole-model STEP). A part is a component whose `Definition::part` holds the part as JSON (`.daedalus` 6
  `part` line) and whose faces are made from it (flat faces whole with holes, bends soft-edged). Tools > Sheet Metal / the
  right-click menu: Make Sheet Metal Part (one face: its outline and holes, circles and arcs drawn in segments becoming
  true arcs; material presets are typical values only), the Flange tool (click a part's edge; height, angle, position from
  the panel), the Sheet Metal panel in the tray with tabs Drawing (outline with size, material, flange list), 3D (bent or
  flat in the model, Flange tool, part STEP) and Flat pattern (blank with red bend lines, size, bend table, DXF in mm or
  inches: lines and true arcs on CUT, bend centre lines on BEND). Double-clicking a part opens the panel, not the
  geometry. File > Export STEP: parts exact, other closed groups/components as exact flat-faced solids (kernel sew), the
  rest named as left out. Flanges bend up or down; hems (180 degrees, the rule's hem gap); the flat part in 3D shows its
  bend lines; shop checks in the Flat pattern tab (rules of thumb, not a shop's limits: hole within 2.5T + R of a bend,
  flange under 4T, radius under T). Checked on screen 2026-11-06 (face -> part -> two flanges -> flat; a hem; blank
  sizes match hand numbers). Convert to Sheet Metal (tool, `sheet/convert.rs` = SolidCraft's sheet_convert.rs with a
  local `ModelBody`): click a face of a solid group/component (stays put), then each bend edge (taken onto the nearest
  edge), Convert in the panel; a shelled solid keeps its walls' thickness; the part keeps the solid (`SolidSource`) and
  rebuilds from it. Checked on screen (an L with 2 mm walls: blank matches hand numbers). Not yet: typing in the panel
  driven on screen (no keys sent to the nested desktop), jogs and other SolidCraft features in the UI.
- Folds, jogs and reliefs (2026-11-07): `SheetPart::add_fold` (SolidCraft's `add_fold`: a bend along a line across a
  panel, centre or mould line on it; the blank keeps its size) and `add_jog` (two folds the opposite way; the second fold's
  distance solved so the far side steps exactly the asked amount, face to face, and stays parallel; a step too small for
  the bends is refused with the least). They are steps in the part's ordered list (`FlangeAsked::fold`, `.daedalus` reads
  old parts unchanged). Tools > Sheet Metal > Fold or Jog (toolbar too): two clicks on a face of the part draw the line;
  the 3D tab sets fold or jog, angle, step, line position, up or down; the Drawing tab lists and edits them. Reliefs in the
  Drawing tab: corner relief None / Round / Square / Tear and its size, bend relief width and depth; new parts start with
  square corner reliefs (what SendCutSend asks for; SolidCraft's default cuts none). Tested in the part model (fold, jogs at
  90/45/60 degrees up and down, every relief shape at a two-flange corner); not yet seen on screen.
- Holes, path flanges, bend table (2026-11-07): Tools > Sheet Metal > Hole or Cut-out (toolbar too): click a flat face of a
  part, bent or not, and a round hole or a rectangle (sides along and across that panel's bend) is cut there, through the flat
  so the blank and the folded part both have it (`SheetPart::add_hole`, `panel_at`). A hole is a step placed on its panel
  (`HoleAsked`: the base's flat x, y, or along a flange's bend and across from where the bend ends), so it moves with the
  panel when a step before it changes; taking away a step renumbers holes on later flanges, and one that would be left on
  nothing stops it. Path flanges: the Flange tool's kind Path, legs typed `20 90, 12 90` (length from the outside of its bend,
  turn; SolidCraft's path flange, one flange on the far edge of the one before). Bend table (`sheet/src/table.rs`): the
  fabricator's CSV (material, thickness, inside radius, K-factor, or `BD` 90 degree bend deduction / `BA` allowance; units
  allowed), chosen with Drawing > Material > Bend table… and kept in `~/.config/daedalus/bend-table.csv`; its rows join the
  material list. Tested in the part model (blank sizes by hand); not yet seen on screen.
- Box selection (2026-11-07, `f52ceb3`): with Select, drag a box. Left to right takes what is wholly inside;
  right to left (dashed, crossing) takes what it touches. Ctrl adds, Shift toggles, Ctrl+Shift takes out. It takes faces,
  edges and loose lines of the geometry being edited, and groups/components whole.
- Stress Test (2026-11-07, `c97cf61`, `a6dc89b`): Tools > Stress Test and its panel (`viewer/src/stressui.rs`). Click a
  closed group or component, then the faces that hold it (blue) and the faces pushed (orange); force in N, kN, kg or lb,
  its direction, own weight, material, cubes along the longest side (a sheet part starts at two cubes across its thickness);
  Run solves on a thread of its own; the result paints the part blue to red, bent by the load (drawn larger). Checked against
  textbook answers (tension F/A and FL/AE; cantilever tip within 1% of PL³/3EI plus shear with two cubes through it; Mc/I
  within 5% halfway, the peak at the support 59-65 MPa against 60), and from the first click to the result on a bar group.
  Cube size chosen by default (`divisions` 0: the coarsest of `daedalus_fea::AUTO` that fills the part's volume to 3%, under
  300 000 cubes). Checked on screen 2026-11-07 on an L bracket (80 x 100 x 60 mm, 2 mm): held by the upright, 100 N down on
  the flat leg: 27 600 cubes of 1 mm, 10 s, 57.8 MPa (hand 58.5), 0.47 mm, 0.217 kg (0.220). Not built: shell elements
  (voxels already bend right at two cubes across a sheet), several load cases, results saved in the file.
- Next milestones (my decision, 2026-11-07: stress testing first, now built): more sheet metal (folds, jogs and reliefs
  done 2026-11-07, as are path flanges, holes on bent panels and bend tables; next there: seeing them on screen, then
  SolidCraft's other features as needed) Constraints are still to come.
- Biggest open decisions: geometry kernel (half-edge mesh vs OpenCASCADE bindings vs pure-Rust B-rep),
  constraint solver, reuse CADCraft crates or reimplement.

## Fabrication facts (SendCutSend, public guidelines)
Flat DXF: 1:1, inches preferred (mm fine if consistent), all cut geometry on one layer, closed paths, no
duplicate lines or stray points, text outlined, a line at the centre of each bend. Bent parts need 3D STEP
using the shop's own K-factor and radius; square bend reliefs. Unverified: whether a separate `BEND` layer
is accepted next to the cut layer.
