//! GPU side of the viewport: shaded faces, edges drawn over them, overlay
//! lines (rubber bands, axes) and the egui interface on top. The face style
//! (SketchUp's Style toolbar) and Back Edges change how faces and hidden edges
//! are drawn.

use bytemuck::{Pod, Zeroable};
use daedalus_core::Mesh;
use glam::Mat4;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const EDGE_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    /// 0.0 normal, 1.0 selected, 2.0 flagged by the inspector.
    highlight: f32,
    /// The face's colour (Paint Bucket), or the default material's.
    color: [f32; 3],
    /// Where on the face's texture, in tiles (it repeats), and which texture layer (-1: none).
    uv: [f32; 2],
    layer: f32,
}

/// Each texture's layer on the GPU is this many pixels square (an image of another size is resized to it).
pub const TEXTURE_SIZE: u32 = 512;

/// The default material: the light grey faces have when nothing is painted on them.
const DEFAULT_FACE: [f32; 3] = [0.78, 0.80, 0.84];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

impl LineVertex {
    /// A line's end at model point `p` (f64; the GPU takes f32).
    pub fn at(p: impl Into<[f64; 3]>, color: [f32; 4]) -> Self {
        LineVertex { position: gpu(p.into()), color }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    light: [f32; 4],
    /// x: how faces are drawn (0 shaded, 1 hidden line, 2 X-ray); y, z, w unused.
    style: [f32; 4],
    /// Section Plane: xyz the normal of the side cut away, w the plane's offset (cut where dot(p, n) > w); zero: no cut.
    section: [f32; 4],
    /// The eye (w = 1), or in parallel projection the direction toward it (w = 0): which side of a face is seen.
    eye: [f32; 4],
}

/// How faces are drawn, as SketchUp's Style toolbar offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceStyle {
    /// Edges only, every one of them, no faces.
    Wireframe,
    /// Faces in the background colour: hidden edges are hidden, nothing is shaded.
    HiddenLine,
    /// Lit faces (the default).
    Shaded,
    /// Faces see-through, so edges behind them show.
    XRay,
    /// Lit faces, all in the default material (paint not shown).
    Monochrome,
    /// Lit faces with their textures drawn (Shaded shows a textured face in its texture's colour).
    Textured,
}

impl FaceStyle {
    pub const ALL: [FaceStyle; 6] =
        [FaceStyle::Wireframe, FaceStyle::HiddenLine, FaceStyle::Shaded, FaceStyle::Textured, FaceStyle::Monochrome, FaceStyle::XRay];

    pub fn label(self) -> &'static str {
        match self {
            FaceStyle::Wireframe => "Wireframe",
            FaceStyle::HiddenLine => "Hidden Line",
            FaceStyle::Shaded => "Shaded",
            FaceStyle::XRay => "X-Ray",
            FaceStyle::Monochrome => "Monochrome",
            FaceStyle::Textured => "Shaded with Textures",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            FaceStyle::Wireframe => "wireframe",
            FaceStyle::HiddenLine => "hidden-line",
            FaceStyle::Shaded => "shaded",
            FaceStyle::XRay => "x-ray",
            FaceStyle::Monochrome => "monochrome",
            FaceStyle::Textured => "textured",
        }
    }

    pub fn from_key(k: &str) -> Option<FaceStyle> {
        FaceStyle::ALL.into_iter().find(|s| s.key() == k)
    }
}

/// The display style: the face style, and whether edges hidden behind faces are drawn dashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub faces: FaceStyle,
    pub back_edges: bool,
}

/// Flat-shaded triangle list: every face gets its own vertices and normal.
/// `tiles`: each texture's tile size (width, height, metres), by number; a face painted with one gets its place on it, the
/// tiles lined up with the face (across: level in the face; up: square to that in it).
fn face_vertices(
    mesh: &Mesh,
    tris: &[Vec<[u32; 3]>],
    selected: &[usize],
    flagged: &[usize],
    faded_from: usize,
    tiles: &[[f64; 2]],
) -> Vec<Vertex> {
    let mut out = Vec::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        let normal = newell(mesh, face).map(|c| c as f32);
        let highlight = if fi >= faded_from {
            3.0
        } else if selected.binary_search(&fi).is_ok() {
            1.0
        } else if flagged.binary_search(&fi).is_ok() {
            2.0
        } else {
            0.0
        };
        let paint = mesh.color_of(fi);
        let color = paint.map_or(DEFAULT_FACE, |c| c.rgb.map(|x| x as f32 / 255.0));
        // the texture's axes on this face: across level (or along x when it lies flat), up square to that
        let texture = paint.and_then(|p| p.texture).filter(|&t| (t as usize) < tiles.len());
        let n = newell(mesh, face);
        let across = if n[2].abs() > 0.999 {
            [1.0, 0.0, 0.0]
        } else {
            let a = [-n[1], n[0], 0.0];
            let l = (a[0] * a[0] + a[1] * a[1]).sqrt();
            [a[0] / l, a[1] / l, 0.0]
        };
        let up = [n[1] * across[2] - n[2] * across[1], n[2] * across[0] - n[0] * across[2], n[0] * across[1] - n[1] * across[0]];
        for &tri in tris.get(fi).map(Vec::as_slice).unwrap_or(&[]) {
            for v in tri {
                let p = mesh.positions[v as usize];
                let (uv, layer) = match texture {
                    Some(t) => {
                        let [w, h] = tiles[t as usize];
                        let dot = |a: [f64; 3]| p[0] * a[0] + p[1] * a[1] + p[2] * a[2];
                        // images run downward; the face's up runs up
                        ([(dot(across) / w.max(1e-9)) as f32, (-dot(up) / h.max(1e-9)) as f32], t as f32)
                    }
                    None => ([0.0, 0.0], -1.0),
                };
                out.push(Vertex { position: gpu(p), normal, highlight, color, uv, layer });
            }
        }
    }
    out
}

/// Edges of what lies outside an open group or component: drawn faint.
const FADED_EDGE: [f32; 4] = [0.62, 0.63, 0.66, 1.0];

/// Selected edges: SketchUp's selection blue, a little deeper than the selected faces' tint so a thin line still shows it.
const SELECTED_EDGE: [f32; 4] = [0.05, 0.33, 0.95, 1.0];

/// `edges`: the mesh's edges to draw (`Mesh::visible_edges`). `selected`: edges drawn in the selection colour, as vertex
/// pairs (lower number first), sorted.
fn edge_vertices(mesh: &Mesh, edges: &[(u32, u32)], faded_vertex: u32, selected: &[[u32; 2]]) -> Vec<LineVertex> {
    edges
        .iter()
        .flat_map(|&(a, b)| {
            let color = if a.min(b) >= faded_vertex {
                FADED_EDGE
            } else if selected.binary_search(&[a.min(b), a.max(b)]).is_ok() {
                SELECTED_EDGE
            } else {
                EDGE_COLOR
            };
            [(a, color), (b, color)]
        })
        .map(|(v, color)| LineVertex { position: gpu(mesh.positions[v as usize]), color })
        .collect()
}

/// A model point (f64) as the GPU takes it (f32).
pub fn gpu(p: [f64; 3]) -> [f32; 3] {
    p.map(|c| c as f32)
}

/// A face's unit normal (Newell's method), in the model's precision.
pub fn newell(mesh: &Mesh, face: &[u32]) -> [f64; 3] {
    let mut n = glam::DVec3::ZERO;
    for i in 0..face.len() {
        let a = glam::DVec3::from(mesh.positions[face[i] as usize]);
        let b = glam::DVec3::from(mesh.positions[face[(i + 1) % face.len()] as usize]);
        n += glam::DVec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    n.normalize_or_zero().into()
}

/// Samples per pixel for the 3D scene: edges and faces are smoothed (anti-aliased) instead of stair-stepped.
const MSAA: u32 = 4;

fn depth_view(device: &wgpu::Device, w: u32, h: u32, samples: u32) -> wgpu::TextureView {
    target_view(device, w, h, samples, DEPTH_FORMAT, "depth")
}

/// A render target of the window's size: the depth buffer, or the multisampled colour buffer the scene is drawn into
/// before it is resolved to the screen.
fn target_view(device: &wgpu::Device, w: u32, h: u32, samples: u32, format: wgpu::TextureFormat, label: &str) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// What egui produced for this frame.
pub struct UiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    face_pipeline: wgpu::RenderPipeline,
    xray_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    back_edge_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// What the bind group is made from, to remake it when the textures change.
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    depth_view: wgpu::TextureView,
    /// Samples per pixel actually used (MSAA, or 1 where the GPU cannot), and the multisampled colour target.
    samples: u32,
    msaa_view: Option<wgpu::TextureView>,
    faces: (wgpu::Buffer, u32),
    edges: (wgpu::Buffer, u32),
    egui: egui_wgpu::Renderer,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Renderer {
        let size = window.inner_size();
        // Vulkan (Metal, DX12) first, and OpenGL only when none of them gives an adapter (or WGPU_BACKEND asks for it).
        // Starting OpenGL alongside for nothing was not free: with NVIDIA's EGL its clean-up at exit ran after the window's
        // Wayland connection was gone and crashed (SIGSEGV in libnvidia-egl-wayland2, 2026-11-06).
        let mut found = None;
        for backends in [wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY), wgpu::Backends::GL] {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends, ..Default::default() });
            let Ok(surface) = instance.create_surface(window.clone()) else { continue };
            let adapter =
                instance.request_adapter(&wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() }).await;
            if let Some(adapter) = adapter {
                found = Some((instance, surface, adapter));
                break;
            }
        }
        let (_instance, surface, adapter) = found.expect("no suitable GPU adapter (Vulkan or OpenGL)");
        let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default(), None).await.expect("request device");
        let config = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface not supported by adapter");
        surface.configure(&device, &config);
        // 4x MSAA where the GPU offers it for the window's format (it nearly always does)
        let samples = if adapter.get_texture_format_features(config.format).flags.sample_count_supported(MSAA)
            && adapter.get_texture_format_features(DEPTH_FORMAT).flags.sample_count_supported(MSAA)
        {
            MSAA
        } else {
            1
        };

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // textures repeat across a face
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("textures"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = texture_bind_group(&device, &queue, &bgl, &uniform_buffer, &sampler, &[]);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let face_shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let line_shader = device.create_shader_module(wgpu::include_wgsl!("lines.wgsl"));
        let target =
            |blend| [Some(wgpu::ColorTargetState { format: config.format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })];
        let opaque = target(wgpu::BlendState::REPLACE);
        let blended = target(wgpu::BlendState::ALPHA_BLENDING);
        let face_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32, 3 => Float32x3, 4 => Float32x2, 5 => Float32],
        };
        // a pair of line vertices (one segment) per instance: the shader turns it into a strip
        let line_layout = wgpu::VertexBufferLayout {
            array_stride: 2 * std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Float32x3, 3 => Float32x4],
        };
        let pipeline = |label,
                        shader: &wgpu::ShaderModule,
                        buffer: &wgpu::VertexBufferLayout,
                        topology,
                        depth,
                        fs: &'static str,
                        targets: &[Option<wgpu::ColorTargetState>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: std::slice::from_ref(buffer),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets,
                }),
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: Some(depth),
                multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
                multiview: None,
                cache: None,
            })
        };
        // Faces are pushed slightly back so their own edges win the depth test.
        let face_pipeline = pipeline(
            "faces",
            &face_shader,
            &face_layout,
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 1.5, clamp: 0.0 },
            },
            "fs_main",
            &opaque,
        );
        // X-ray faces blend over what is behind them and hide nothing
        let xray_pipeline = pipeline(
            "x-ray faces",
            &face_shader,
            &face_layout,
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 1.5, clamp: 0.0 },
            },
            "fs_main",
            &blended,
        );
        let edge_pipeline = pipeline(
            "edges",
            &line_shader,
            &line_layout,
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            },
            "fs_main",
            &blended,
        );
        // Back Edges: the edges that faces hide, dashed and grey
        let back_edge_pipeline = pipeline(
            "back edges",
            &line_shader,
            &line_layout,
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Greater,
                stencil: Default::default(),
                bias: Default::default(),
            },
            "fs_dashed",
            &blended,
        );
        let overlay_pipeline = pipeline(
            "overlay",
            &line_shader,
            &line_layout,
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            },
            "fs_main",
            &blended,
        );

        let empty = |device: &wgpu::Device| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 64,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            })
        };
        Renderer {
            depth_view: depth_view(&device, config.width, config.height, samples),
            msaa_view: (samples > 1).then(|| target_view(&device, config.width, config.height, samples, config.format, "msaa")),
            samples,
            faces: (empty(&device), 0),
            edges: (empty(&device), 0),
            egui: egui_wgpu::Renderer::new(&device, config.format, None, 1, false),
            surface,
            device,
            queue,
            config,
            face_pipeline,
            xray_pipeline,
            edge_pipeline,
            back_edge_pipeline,
            overlay_pipeline,
            uniform_buffer,
            bind_group,
            bind_layout: bgl,
            sampler,
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = depth_view(&self.device, w, h, self.samples);
        self.msaa_view = (self.samples > 1).then(|| target_view(&self.device, w, h, self.samples, self.config.format, "msaa"));
    }

    /// Uploads the model after an edit, a selection change or a preview step.
    /// `faded`: faces from this number on, and edges between vertices from that number on, lie outside the open group
    /// or component and are drawn faded.
    /// `tris`: each face's triangles (`Mesh::face_triangles`), and `edges` its edges to draw (`Mesh::visible_edges`), kept
    /// by the caller so a highlight change does not work them out again.
    /// `selected_edges`: edges drawn in the selection colour (vertex pairs, lower number first, sorted).
    /// `tiles`: each texture's tile size, by number (see `face_vertices`).
    #[allow(clippy::too_many_arguments)]
    pub fn set_scene(
        &mut self,
        mesh: &Mesh,
        tris: &[Vec<[u32; 3]>],
        edges: &[(u32, u32)],
        selected: &[usize],
        selected_edges: &[[u32; 2]],
        flagged: &[usize],
        faded: (usize, u32),
        tiles: &[[f64; 2]],
    ) {
        let faces = face_vertices(mesh, tris, selected, flagged, faded.0, tiles);
        let edges = edge_vertices(mesh, edges, faded.1, selected_edges);
        self.faces = (self.buffer(bytemuck::cast_slice(&faces)), faces.len() as u32);
        self.edges = (self.buffer(bytemuck::cast_slice(&edges)), edges.len() as u32);
    }

    /// The textures faces may be painted with, by number: each `TEXTURE_SIZE` square, RGBA, rows top first.
    pub fn set_textures(&mut self, layers: &[Vec<u8>]) {
        self.bind_group = texture_bind_group(&self.device, &self.queue, &self.bind_layout, &self.uniform_buffer, &self.sampler, layers);
    }

    fn buffer(&self, contents: &[u8]) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: if contents.is_empty() { &[0; 64] } else { contents },
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    /// Draws one frame. `guides` (the axes) are hidden by geometry like edges;
    /// `overlay` lines (rubber bands) are drawn on top of everything.
    /// `line_width` is in pixels (scaled for HiDPI by the caller).
    /// `section`: a section plane as (point on it, normal of the side cut away), or None.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        view_proj: Mat4,
        eye: [f32; 4],
        style: Style,
        line_width: f32,
        section: Option<([f32; 3], [f32; 3])>,
        guides: &[LineVertex],
        overlay: &[LineVertex],
        ui: UiFrame,
    ) {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            Err(e) => {
                eprintln!("surface error: {e}");
                return;
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mode = match style.faces {
            FaceStyle::HiddenLine => 1.0,
            FaceStyle::XRay => 2.0,
            FaceStyle::Monochrome => 3.0,
            FaceStyle::Textured => 4.0,
            _ => 0.0,
        };
        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            light: [0.4, -0.6, 0.9, 0.0],
            style: [mode, self.config.width as f32, self.config.height as f32, line_width],
            section: section.map_or([0.0; 4], |(o, n)| [n[0], n[1], n[2], o[0] * n[0] + o[1] * n[1] + o[2] * n[2]]),
            eye,
        };
        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        let guide_buffer = self.buffer(bytemuck::cast_slice(guides));
        let overlay_buffer = self.buffer(bytemuck::cast_slice(overlay));

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    // drawn multisampled, then resolved onto the screen
                    view: self.msaa_view.as_ref().unwrap_or(&view),
                    resolve_target: self.msaa_view.as_ref().map(|_| &view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.93, g: 0.94, b: 0.96, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            if matches!(style.faces, FaceStyle::HiddenLine | FaceStyle::Shaded | FaceStyle::Textured | FaceStyle::Monochrome) {
                pass.set_pipeline(&self.face_pipeline);
                pass.set_vertex_buffer(0, self.faces.0.slice(..));
                pass.draw(0..self.faces.1, 0..1);
            }
            pass.set_pipeline(&self.edge_pipeline);
            pass.set_vertex_buffer(0, self.edges.0.slice(..));
            pass.draw(0..6, 0..self.edges.1 / 2);
            if style.back_edges
                && matches!(style.faces, FaceStyle::HiddenLine | FaceStyle::Shaded | FaceStyle::Textured | FaceStyle::Monochrome)
            {
                pass.set_pipeline(&self.back_edge_pipeline);
                pass.draw(0..6, 0..self.edges.1 / 2);
            }
            if style.faces == FaceStyle::XRay {
                pass.set_pipeline(&self.xray_pipeline);
                pass.set_vertex_buffer(0, self.faces.0.slice(..));
                pass.draw(0..self.faces.1, 0..1);
            }
            pass.set_pipeline(&self.edge_pipeline);
            pass.set_vertex_buffer(0, guide_buffer.slice(..));
            pass.draw(0..6, 0..guides.len() as u32 / 2);
            pass.set_pipeline(&self.overlay_pipeline);
            pass.set_vertex_buffer(0, overlay_buffer.slice(..));
            pass.draw(0..6, 0..overlay.len() as u32 / 2);
        }

        let screen =
            egui_wgpu::ScreenDescriptor { size_in_pixels: [self.config.width, self.config.height], pixels_per_point: ui.pixels_per_point };
        for (id, delta) in &ui.textures.set {
            self.egui.update_texture(&self.device, &self.queue, *id, delta);
        }
        let extra = self.egui.update_buffers(&self.device, &self.queue, &mut encoder, &ui.primitives, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.egui.render(&mut pass, &ui.primitives, &screen);
        }
        for id in &ui.textures.free {
            self.egui.free_texture(id);
        }
        self.queue.submit(extra.into_iter().chain(Some(encoder.finish())));
        frame.present();
    }
}

#[cfg(test)]
mod shader_tests {
    /// Every shader parses and validates as wgpu would at start (a broken one would only show as a crash at launch).
    #[test]
    fn the_shaders_compile() {
        for (name, src) in [("shader.wgsl", include_str!("shader.wgsl")), ("lines.wgsl", include_str!("lines.wgsl"))] {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }
}

/// The bind group: the uniforms, the textures as layers of one array (a blank layer when there are none), the sampler.
fn texture_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    sampler: &wgpu::Sampler,
    layers: &[Vec<u8>],
) -> wgpu::BindGroup {
    let count = layers.len().max(1) as u32;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("textures"),
        size: wgpu::Extent3d { width: TEXTURE_SIZE, height: TEXTURE_SIZE, depth_or_array_layers: count },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let want = (TEXTURE_SIZE * TEXTURE_SIZE * 4) as usize;
    for (i, data) in layers.iter().enumerate() {
        if data.len() != want {
            continue;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: i as u32 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * TEXTURE_SIZE), rows_per_image: Some(TEXTURE_SIZE) },
            wgpu::Extent3d { width: TEXTURE_SIZE, height: TEXTURE_SIZE, depth_or_array_layers: 1 },
        );
    }
    let view =
        texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}
