# Daedalus CAD

A SketchUp-style modeler in Rust, built from first principles (no code derived from other applications).

## Goals
One multi-purpose CAD tool: SketchUp-style 3D modeling for buildings and layouts, top-down 2D
drafting, and mechanical part design including sheet metal (flat patterns and bend lines for
laser/waterjet and bending services). SketchUp's tools and controls are the model for the UX.

## Layout
- `daedalus-core`: `Mesh` (interchange), `HalfEdgeMesh` (topology checks, normals, `split_face`,
  `push_pull`, `pick_face`) and `Model` (document with atomic undo/redo)
- `daedalus-io`: format importers/exporters behind `MeshImporter` / `MeshExporter`
- `daedalus-app`: CLI that prints mesh stats
- `daedalus-viewer`: wgpu viewport. `cargo run -p daedalus-viewer -- model.stl`
  - left-drag orbit, right/middle-drag pan, wheel zoom
  - click selects a face, Up/Down push/pulls it, Ctrl+Z / Ctrl+Y undo/redo, Esc deselects
  - `I` inspects the solid, `F` repairs it
  - no argument starts with a rectangle on the ground; imports with non-manifold
    topology open read-only until repaired

## Solid inspector
Checks whether a model is a valid solid (open borders, non-manifold edges, flipped or inverted
faces, duplicate and zero-area faces, short edges, stray vertices) and repairs what it can.
```sh
cargo run -p daedalus-app -- inspect model.stl                 # report; exit code 1 if not a solid
cargo run -p daedalus-app -- inspect model.stl --fix fixed.stl # repair and write
```
In the viewer press `I` to inspect (problem faces turn red) and `F` to repair.

## Sheet metal flat patterns
```sh
# L-bracket: 50 and 30 outside flanges, one 90 degree bend with 2.0 inside radius, 2 mm sheet
cargo run -p daedalus-app -- flatten bracket.dxf --thickness 2 --k 0.44 --width 40 50 90:2 30
```
Writes a 1:1 DXF with the outline on layer `CUT` and a line at the centre of each bend on layer
`BEND`. K-factor and bend radius come from your fabricator for the material and thickness.
Flange lengths are outside dimensions; use a negative angle for a bend down.

## Building on Arch Linux
```sh
sudo pacman -S --needed base-devel git rustup vulkan-icd-loader mesa libxkbcommon wayland libx11 libxcursor libxrandr libxi
# plus a Vulkan driver for your GPU: vulkan-radeon, vulkan-intel or nvidia-utils
rustup default stable
cargo run -p daedalus-viewer
```
winit loads the X11/Wayland libraries at runtime, so the viewer runs on either session type.
The workspace is pure Rust with no C build dependencies beyond a linker (`base-devel`).
CI (`.github/workflows/ci.yml`) builds and tests on Arch and Windows.

## Roadmap
Behavior targets come from the publicly documented feature set of other free modelers
(for example IngeTrazo, GPL-3.0). Daedalus is designed and written independently;
no source from those projects is used.

1. Drawing: line, rectangle, circle, polygon, arc; edge splitting of faces; inference and axis locks
2. Value-box entry of exact lengths; repeat-last-command
3. Modify: move, rotate, scale, offset, follow-me, eraser, hide
4. Groups and components (shared definitions), layers, scenes
5. Materials and styles, edge rendering, section planes
6. Solid operations (union, subtract, intersect) and a watertightness guard
7. Native save format, then SKP read via the SDK, glTF export
8. Sheet layout with vector PDF/DXF output

## Format status
| Format | Status | Notes |
|---|---|---|
| STL, OBJ | import + export | hand-written parsers |
| DXF | import + export | ASCII only; `3DFACE` and polyface meshes |
| DAE | import + export | triangles, polylist, polygons; no materials or transforms |
| JPEG/PNG/TIFF/BMP/DIB | load | `image` crate |
| PSD | load | flattened composite only (`psd` crate) |
| PDF | todo | `pdfium-render` needs the pdfium library at runtime |
| FBX | todo | `fbxcel` |
| SKP | todo | Trimble SketchUp C SDK (check Linux binaries), feature-gated |
| DWG | todo | LibreDWG is GPLv3, so isolate in an optional crate |
| F3D/F3Z/CAM360 | blocked | no public spec; support STEP instead |
| RBZ | later | needs a reimplementation of the public SketchUp Ruby API |

## Next
Line/rectangle/push-pull tools on `HalfEdgeMesh`, picking and snapping, edge rendering,
undo/redo command stack, native `.daedalus` save format.
