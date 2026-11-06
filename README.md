# Daedalus CAD

A SketchUp-style modeler in Rust, built from first principles (no code derived from other applications).

## Goals
One multi-purpose CAD tool: SketchUp-style 3D modeling for buildings and layouts, top-down 2D
drafting, and mechanical part design including sheet metal (flat patterns and bend lines for
laser/waterjet and bending services). SketchUp's tools and controls are the model for the UX.

## Layout
- `daedalus-core`: `Mesh` (interchange), `HalfEdgeMesh` (topology checks, normals, `split_face`,
  `push_pull`, `erase_face`), `Model` (document with atomic undo/redo), `sketch` (workplanes,
  rectangle, circle, polygon) and `triangulate` (ear clipping for concave faces)
- `daedalus-io`: format importers/exporters behind `MeshImporter` / `MeshExporter`
- `daedalus-app`: CLI that prints mesh stats
- `daedalus-viewer`: the modeler (wgpu + egui). `cargo run -p daedalus-viewer [-- model.stl]`

## Using the modeler
Laid out like SketchUp: menus and toolbar on top, hints and the Measurements box at the bottom.

| Tool | Key | Use |
|---|---|---|
| Select | Space | click a face; Delete erases it |
| Line | L | click corners; click the start point to close the loop into a face |
| Rectangle | R | two corners, or type `width,height` after the first |
| Circle | C | centre, then radius (click or type); type `24s` to set segments |
| Polygon | | centre, then radius; type `6s` to set sides |
| Push/Pull | P | click a face, move, click; or type a distance. Typing again right after re-does it |
| Eraser | E | click a face |
| Orbit / Pan (Hand) / Zoom | O / H / Z | left-drag with the tool; Shift+Z zooms to extents |

- Mouse: middle-drag orbits, Shift+middle-drag (or right-drag) pans, the wheel zooms toward the cursor.
- Typing digits goes straight into the Measurements box; Enter applies, Esc clears or cancels.
- Points snap to endpoints and midpoints; lines lock to the red/green axes within 3 degrees.
  Shapes are drawn on the face under the first click, or on the ground.
- Camera menu: Top, Iso, Parallel Projection. Ctrl+Z / Ctrl+Y undo and redo.
- `I` inspects the solid, Tools > Repair Solid repairs it (`F` is Offset, as in SketchUp). Imports with non-manifold topology open read-only until repaired.

## Solid inspector
Checks whether a model is a valid solid (open borders, non-manifold edges, flipped or inverted
faces, duplicate and zero-area faces, short edges, stray vertices) and repairs what it can.
```sh
cargo run -p daedalus-app -- inspect model.stl                 # report; exit code 1 if not a solid
cargo run -p daedalus-app -- inspect model.stl --fix fixed.stl # repair and write
```
In the viewer press `I` to inspect (problem faces turn red) and use Tools > Repair Solid to repair.

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
**On Labyrinth OS** Daedalus CAD is in Agora, the app store (Create > 3D & CAD): one click installs the signed build from
Labyrinth's package repository (`daedalus-cad-git`: `daedalus` and `daedalus-cli`, with the application-menu entry and icon from
`packaging/`), and Agora's updates keep it current.

To install it for your user (binary in `~/.local/bin/daedalus`, plus an application-menu entry and icon):
```sh
packaging/install.sh              # run again after `git pull` to update
packaging/install.sh --uninstall
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

## Coplanar faces
As in SketchUp, neighbouring faces that lie in one plane are one face: pushing a face of a solid again leaves no seam, and
Repair (Tools > Repair Solid) turns an imported mesh's triangle pairs back into the faces they were drawn as. Tools > Merge Coplanar Faces does
it by hand. A group whose outline would have a hole is left as it is until faces with holes are supported.

## Toolbar
Icons in SketchUp's manner (a pencil for Line, a red box with an arrow for Push/Pull, the red and green orbit arrows), each
with its name and shortcut on hover. View > Toolbar puts the toolbar across the top or down the left side; the choice is
remembered.

## Units
The Units menu shows and takes lengths in millimetres, centimetres, metres, inches, or feet and inches (to 1/16"). A bare number
is in the chosen unit; a unit written after it always wins, so `300mm`, `1.2m`, `6"`, `4'` or `4' 6 1/2"` work whatever is shown.
The choice is remembered (`~/.config/daedalus/settings.conf`). The model itself is kept in metres.

## Saving
File > Save writes Daedalus's own `.daedalus` file: plain text (`v x y z` per vertex, `f` and vertex numbers per face) with
a version on the first line, so it is readable, diffable and kept bit-exact. Open, Save As, Import (STL, OBJ, DXF, DAE) and
Export use a file browser built into the window. The title shows a dot while there are unsaved changes, and New, Open,
Import or closing the window asks before throwing them away.

## Next
Imprinting (a shape drawn on a face splits it instead of overlapping it), vertex welding so shapes that touch share edges,
arcs, and Move/Rotate/Offset.

## License

MPL-2.0 (see `LICENSE`), like Labyrinth OS's other from-scratch projects. Daedalus is written independently; no source from other
modelers is used.
