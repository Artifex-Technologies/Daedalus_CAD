"""Converts my SketchUp models (.skp) so they can be opened without SketchUp, using OpenSKP (MIT, reverse-engineered
reader; installed in ~/.local/share/daedalus/openskp). The .skp files are only read, never changed.

For every .skp under the folder given (default: ~/3D Objects), it writes into "<folder>/Converted from SketchUp/", keeping
the same subfolders:
  NAME.obj + NAME.mtl   the model's shape and materials, every group and component put in place, metres, Z up
                        (Daedalus: File > Import)
  NAME.dskp             the model for Daedalus with its groups and components (Daedalus opens it, or the .skp itself)
  NAME.json             the whole model as OpenSKP reads it: definitions (groups, components), instances, faces, edges,
                        layers, materials; what a SketchUp importer for Daedalus is built from
and a report, "conversion report.txt", saying for each file what was found or what went wrong.

Run:  skp-convert            (or: skp-convert "/some/folder"; Daedalus runs: skp-convert --one FILE.skp OUT.dskp)
"""
import pathlib
import sys
import time
import traceback

from openskp import SkpFile
from openskp.export import json_export, obj

OUT_NAME = "Converted from SketchUp"


def z_up(path: pathlib.Path) -> None:
    """OpenSKP's OBJ is Y-up (glTF's frame, metres); Daedalus and SketchUp are Z-up: (x, y, z) becomes (x, -z, y), a turn
    (faces keep their winding)."""
    out = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith(("v ", "vn ")):
            tag, x, y, z, *rest = line.split()
            fy, fz = float(y), float(z)
            line = " ".join([tag, x, f"{-fz:.6f}", f"{fy:.6f}", *rest])
        out.append(line)
    path.write_text("\n".join(out) + "\n", encoding="utf-8")


INCH = 0.0254


def _q(t: str) -> str:
    return '"' + t.replace("\\", "\\\\").replace('"', '\\"').replace("\n", " ") + '"'


def _block(lines: list, d) -> None:
    """One definition's geometry (or the top level's) as .dskp lines: vertices in metres, faces as loops of vertex numbers
    (the outer loop first), loose and soft edges, and the instances placed in it."""
    index = {}
    for vid, v in d.vertices.items():
        index[vid] = len(index)
        lines.append(f"v {v.x * INCH!r} {v.y * INCH!r} {v.z * INCH!r}")
    used = set()
    for f in d.faces.values():
        rings = []
        for loop in f.loops:
            ring = []
            for edge_id, orient in loop:
                e = d.edges.get(edge_id)
                if e is None:
                    ring = []
                    break
                used.add(edge_id)
                a, b = (e.v1_id, e.v2_id) if orient >= 0 else (e.v2_id, e.v1_id)
                ring.append(index.get(a, -1))
            if len(ring) >= 3 and all(i >= 0 for i in ring):
                rings.append(ring)
        if not rings:
            continue
        n = f.normal or (0.0, 0.0, 0.0)
        mat = f.material_id if f.material_id is not None else (f.back_material_id if f.back_material_id is not None else -1)
        parts = [f"f {mat} {n[0]!r} {n[1]!r} {n[2]!r} {len(rings)}"]
        for r in rings:
            parts.append(str(len(r)))
            parts.extend(map(str, r))
        lines.append(" ".join(parts))
    for eid, e in d.edges.items():
        a, b = index.get(e.v1_id), index.get(e.v2_id)
        if a is None or b is None or a == b:
            continue
        if eid not in used:
            if not e.hidden:
                lines.append(f"l {a} {b}")
        elif e.soft or e.smooth or e.hidden:
            lines.append(f"s {a} {b}")
    for inst in d.instances:
        m = list(inst.matrix) + [0.0] * 13
        sc = m[12] if m[12] else 1.0
        rows = [[sc * m[0], sc * m[1], sc * m[2], sc * m[9] * INCH], [sc * m[3], sc * m[4], sc * m[5], sc * m[10] * INCH], [sc * m[6], sc * m[7], sc * m[8], sc * m[11] * INCH]]
        lines.append("i " + str(inst.ref_idx) + " " + " ".join(repr(x) for row in rows for x in row))


def write_dskp(model, path: pathlib.Path) -> None:
    """The model for Daedalus (its SketchUp importer, daedalus-io/src/skp.rs): a plain text file, version 1."""
    lines = ["daedalus-skp 1", "# from OpenSKP: metres, Z up; materials, then one block per definition, the top level last"]
    for mid, mat in getattr(model, "materials_by_id", {}).items():
        c = mat.color
        lines.append(f"material {mid} {c[0]} {c[1]} {c[2]} {c[3] if len(c) > 3 else 255} {_q(mat.name)}")
    for did, d in model.definitions.items():
        kind = "group" if getattr(d, "is_group", False) else "component"
        lines.append(f"def {did} {kind} {_q(d.name or ('Group' if kind == 'group' else 'Component'))}")
        _block(lines, d)
        lines.append("end")
    lines.append("top")
    _block(lines, model.root)
    lines.append("end")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def convert(src: pathlib.Path, dst_dir: pathlib.Path) -> str:
    dst_dir.mkdir(parents=True, exist_ok=True)
    t = time.time()
    skp = SkpFile.open(src)
    model = skp.parse()
    scene = skp.build_scene()
    obj.export(scene, dst_dir / f"{src.stem}.obj")
    z_up(dst_dir / f"{src.stem}.obj")
    json_export.export(model, dst_dir / f"{src.stem}.json", scene=scene)
    write_dskp(model, dst_dir / f"{src.stem}.dskp")
    faces = len(model.root.faces) + sum(len(getattr(d, "faces", None) or []) for d in model.definitions.values())
    return (
        f"ok   {src.name}: SketchUp {getattr(model, 'version', '?')}, {len(model.definitions)} definitions, "
        f"{len(model.layers)} layers, {len(model.materials)} materials, {faces} faces "
        f"({time.time() - t:.1f} s)"
    )


def one(src: pathlib.Path, dst: pathlib.Path) -> None:
    """Only the .dskp of one file (what Daedalus asks for when a .skp is opened in it)."""
    model = SkpFile.open(src).parse()
    dst.parent.mkdir(parents=True, exist_ok=True)
    write_dskp(model, dst)


def main() -> None:
    if len(sys.argv) == 4 and sys.argv[1] == "--one":
        one(pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3]))
        return
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path.home() / "3D Objects").expanduser()
    out_root = root / OUT_NAME
    files = sorted(p for p in root.rglob("*") if p.suffix.lower() == ".skp" and OUT_NAME not in p.parts)
    if not files:
        print(f"No .skp files under {root}")
        return
    lines = [f"SketchUp conversion, {time.strftime('%Y-%m-%d %H:%M')}, {len(files)} files under {root}", ""]
    ok = 0
    for i, f in enumerate(files, 1):
        rel = f.parent.relative_to(root)
        print(f"[{i}/{len(files)}] {rel / f.name} ...", flush=True)
        try:
            line = convert(f, out_root / rel)
            ok += 1
        except Exception as e:  # one bad file must not stop the rest
            line = f"FAIL {rel / f.name}: {type(e).__name__}: {e}"
            (out_root / rel).mkdir(parents=True, exist_ok=True)
            (out_root / rel / f"{f.stem}.error.txt").write_text(traceback.format_exc())
        print("   ", line, flush=True)
        lines.append(line)
    lines += ["", f"{ok} of {len(files)} converted. The .skp files were not changed."]
    out_root.mkdir(parents=True, exist_ok=True)
    (out_root / "conversion report.txt").write_text("\n".join(lines) + "\n")
    print(f"\n{ok} of {len(files)} converted into {out_root}")


if __name__ == "__main__":
    main()
