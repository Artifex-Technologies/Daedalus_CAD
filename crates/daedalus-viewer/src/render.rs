//! GPU side of the viewport: shaded faces, edges drawn over them, overlay
//! lines (rubber bands, axes) and the egui interface on top. The face style
//! (SketchUp's Style toolbar) and Back Edges change how faces and hidden edges
//! are drawn.

use bytemuck::{Pod, Zeroable};
use daedalus_core::Mesh;
use glam::{Mat4, Vec3};
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
}

/// The default material: the light grey faces have when nothing is painted on them.
const DEFAULT_FACE: [f32; 3] = [0.78, 0.80, 0.84];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
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
}

impl FaceStyle {
    pub const ALL: [FaceStyle; 5] = [FaceStyle::Wireframe, FaceStyle::HiddenLine, FaceStyle::Shaded, FaceStyle::Monochrome, FaceStyle::XRay];

    pub fn label(self) -> &'static str {
        match self {
            FaceStyle::Wireframe => "Wireframe",
            FaceStyle::HiddenLine => "Hidden Line",
            FaceStyle::Shaded => "Shaded",
            FaceStyle::XRay => "X-Ray",
            FaceStyle::Monochrome => "Monochrome",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            FaceStyle::Wireframe => "wireframe",
            FaceStyle::HiddenLine => "hidden-line",
            FaceStyle::Shaded => "shaded",
            FaceStyle::XRay => "x-ray",
            FaceStyle::Monochrome => "monochrome",
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
fn face_vertices(mesh: &Mesh, selected: &[usize], flagged: &[usize], faded_from: usize) -> Vec<Vertex> {
    let mut out = Vec::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        let normal = newell(mesh, face);
        let highlight = if fi >= faded_from {
            3.0
        } else if selected.binary_search(&fi).is_ok() {
            1.0
        } else if flagged.binary_search(&fi).is_ok() {
            2.0
        } else {
            0.0
        };
        let color = mesh.color_of(fi).map_or(DEFAULT_FACE, |c| c.map(|x| x as f32 / 255.0));
        for tri in daedalus_core::triangulate::triangulate(&mesh.positions, face) {
            for v in tri {
                out.push(Vertex { position: mesh.positions[v as usize], normal, highlight, color });
            }
        }
    }
    out
}

/// Edges of what lies outside an open group or component: drawn faint.
const FADED_EDGE: [f32; 4] = [0.62, 0.63, 0.66, 1.0];

fn edge_vertices(mesh: &Mesh, faded_vertex: u32) -> Vec<LineVertex> {
    mesh.visible_edges()
        .into_iter()
        .flat_map(|(a, b)| {
            let color = if a.min(b) >= faded_vertex { FADED_EDGE } else { EDGE_COLOR };
            [(a, color), (b, color)]
        })
        .map(|(v, color)| LineVertex { position: mesh.positions[v as usize], color })
        .collect()
}

pub fn newell(mesh: &Mesh, face: &[u32]) -> [f32; 3] {
    let mut n = Vec3::ZERO;
    for i in 0..face.len() {
        let a = Vec3::from(mesh.positions[face[i] as usize]);
        let b = Vec3::from(mesh.positions[face[(i + 1) % face.len()] as usize]);
        n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
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
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).expect("create surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .expect("no suitable GPU adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default(), None)
            .await
            .expect("request device");
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
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
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let face_shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let line_shader = device.create_shader_module(wgpu::include_wgsl!("lines.wgsl"));
        let target = |blend| {
            [Some(wgpu::ColorTargetState { format: config.format, blend: Some(blend), write_mask: wgpu::ColorWrites::ALL })]
        };
        let opaque = target(wgpu::BlendState::REPLACE);
        let blended = target(wgpu::BlendState::ALPHA_BLENDING);
        let face_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32, 3 => Float32x3],
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
    pub fn set_scene(&mut self, mesh: &Mesh, selected: &[usize], flagged: &[usize], faded: (usize, u32)) {
        let faces = face_vertices(mesh, selected, flagged, faded.0);
        let edges = edge_vertices(mesh, faded.1);
        self.faces = (self.buffer(bytemuck::cast_slice(&faces)), faces.len() as u32);
        self.edges = (self.buffer(bytemuck::cast_slice(&edges)), edges.len() as u32);
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
            _ => 0.0,
        };
        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            light: [0.4, -0.6, 0.9, 0.0],
            style: [mode, self.config.width as f32, self.config.height as f32, line_width],
            section: section.map_or([0.0; 4], |(o, n)| [n[0], n[1], n[2], o[0] * n[0] + o[1] * n[1] + o[2] * n[2]]),
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
            if matches!(style.faces, FaceStyle::HiddenLine | FaceStyle::Shaded | FaceStyle::Monochrome) {
                pass.set_pipeline(&self.face_pipeline);
                pass.set_vertex_buffer(0, self.faces.0.slice(..));
                pass.draw(0..self.faces.1, 0..1);
            }
            pass.set_pipeline(&self.edge_pipeline);
            pass.set_vertex_buffer(0, self.edges.0.slice(..));
            pass.draw(0..6, 0..self.edges.1 / 2);
            if style.back_edges && matches!(style.faces, FaceStyle::HiddenLine | FaceStyle::Shaded | FaceStyle::Monochrome) {
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

        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: ui.pixels_per_point,
        };
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
