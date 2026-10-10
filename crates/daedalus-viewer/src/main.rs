mod camera;

use bytemuck::{Pod, Zeroable};
use camera::Camera;
use daedalus_core::inspect::{self, DEFAULT_TOLERANCE};
use daedalus_core::{HalfEdgeMesh, Mesh, Model};
use glam::Vec3;
use std::path::PathBuf;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const HELP: &str = "\
Left-drag: orbit    Right/middle-drag: pan    Wheel: zoom
Click: select face  Up/Down: push/pull selected face
Ctrl+Z / Ctrl+Y: undo / redo    Esc: deselect
I: inspect solid (problem faces turn red)    F: repair (also makes imports editable)";

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    /// 0.0 normal, 1.0 selected, 2.0 flagged by the inspector.
    highlight: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    light: [f32; 4],
}

/// Flat-shaded triangle list: every face gets its own vertices and normal.
fn build_vertices(mesh: &Mesh, selected: Option<usize>, flagged: &[usize]) -> Vec<Vertex> {
    let mut out = Vec::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        let normal = newell(mesh, face);
        let highlight = if selected == Some(fi) {
            1.0
        } else if flagged.binary_search(&fi).is_ok() {
            2.0
        } else {
            0.0
        };
        for i in 1..face.len() - 1 {
            for &v in &[face[0], face[i], face[i + 1]] {
                out.push(Vertex { position: mesh.positions[v as usize], normal, highlight });
            }
        }
    }
    out
}

fn newell(mesh: &Mesh, face: &[u32]) -> [f32; 3] {
    let mut n = Vec3::ZERO;
    for i in 0..face.len() {
        let a = Vec3::from(mesh.positions[face[i] as usize]);
        let b = Vec3::from(mesh.positions[face[(i + 1) % face.len()] as usize]);
        n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    n.normalize_or_zero().into()
}

/// Starting scene when no file is given: a 2 x 2 rectangle on the ground, ready to push/pull.
fn starter_scene() -> Mesh {
    let mut h = HalfEdgeMesh::default();
    h.add_polygon(&[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 2.0, 0.0], [0.0, 2.0, 0.0]])
        .expect("starter rectangle");
    h.to_mesh()
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
    camera: Camera,
    cursor: (f32, f32),
    left_down: bool,
    pan_down: bool,
    press_at: (f32, f32),
    ctrl: bool,
    model: Model,
    /// Set when an import has non-manifold topology: shown, but not editable.
    readonly: Option<Mesh>,
    selected: Option<usize>,
    /// Sorted face indices flagged by the last inspection.
    flagged: Vec<usize>,
}

fn depth_view(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

impl State {
    async fn new(window: Arc<Window>, mesh: &Mesh) -> State {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).expect("create surface");
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

        let (model, readonly) = match HalfEdgeMesh::from_mesh(mesh) {
            Ok(h) => (Model::from_mesh(h), None),
            Err(e) => {
                eprintln!("note: model is read-only ({e})");
                (Model::new(), Some(mesh.clone()))
            }
        };
        let vertices = build_vertices(mesh, None, &[]);
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

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

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        let camera = match mesh.bounds() {
            Some((lo, hi)) => Camera::fit(Vec3::from(lo), Vec3::from(hi)),
            None => Camera::fit(Vec3::splat(-1.0), Vec3::splat(1.0)),
        };

        State {
            depth_view: depth_view(&device, config.width, config.height),
            window,
            surface,
            device,
            queue,
            config,
            pipeline,
            vertex_buffer,
            vertex_count: vertices.len() as u32,
            uniform_buffer,
            bind_group,
            camera,
            cursor: (0.0, 0.0),
            left_down: false,
            pan_down: false,
            press_at: (0.0, 0.0),
            ctrl: false,
            model,
            readonly,
            selected: None,
            flagged: Vec::new(),
        }
    }

    /// Reports solid problems on stdout and marks the offending faces red.
    fn inspect(&mut self) {
        let report = inspect::inspect(&self.current_mesh(), DEFAULT_TOLERANCE);
        println!("--- solid inspector ---\n{}", report.summary());
        self.flagged = report.flagged_faces();
        self.refresh();
    }

    /// Repairs the model. A repaired import that is manifold becomes editable.
    fn repair(&mut self) {
        let (fixed, summary) = inspect::repair(&self.current_mesh(), DEFAULT_TOLERANCE);
        if !summary.changed_anything() {
            println!("repair: nothing to fix");
            return;
        }
        println!("repair: {summary:?}");
        match HalfEdgeMesh::from_mesh(&fixed) {
            Ok(h) => {
                self.model = Model::from_mesh(h);
                self.readonly = None;
                println!("repair: model is now editable");
            }
            Err(e) => {
                self.readonly = Some(fixed);
                println!("repair: still read-only ({e})");
            }
        }
        self.selected = None;
        self.flagged.clear();
        self.refresh();
    }

    fn current_mesh(&self) -> Mesh {
        match &self.readonly {
            Some(m) => m.clone(),
            None => self.model.mesh().to_mesh(),
        }
    }

    /// Rebuilds GPU geometry after an edit or a selection change.
    fn refresh(&mut self) {
        let vertices = build_vertices(&self.current_mesh(), self.selected, &self.flagged);
        self.vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.vertex_count = vertices.len() as u32;
        self.window.request_redraw();
    }

    fn pick(&mut self, x: f32, y: f32) {
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let (origin, dir) = self.camera.ray(x, y, w, h);
        self.selected = self
            .current_mesh_hit(origin, dir)
            .filter(|&f| f < self.current_mesh().faces.len());
        self.refresh();
    }

    fn current_mesh_hit(&self, origin: Vec3, dir: Vec3) -> Option<usize> {
        let hit = match &self.readonly {
            Some(m) => HalfEdgeMesh::from_mesh(m).ok()?.pick_face(origin.into(), dir.into()),
            None => self.model.mesh().pick_face(origin.into(), dir.into()),
        };
        hit.map(|(f, _)| f)
    }

    fn push_pull(&mut self, direction: f32) {
        if self.readonly.is_some() {
            eprintln!("this model is read-only");
            return;
        }
        let Some(face) = self.selected else { return };
        let step = self.camera.distance * 0.03 * direction;
        self.flagged.clear(); // face indices change with every edit
        match self.model.push_pull(face, step) {
            Ok(new_face) => self.selected = Some(new_face),
            Err(e) => eprintln!("push/pull failed: {e}"),
        }
        self.refresh();
    }

    fn history(&mut self, undo: bool) {
        if self.readonly.is_some() {
            return;
        }
        let changed = if undo { self.model.undo() } else { self.model.redo() };
        if changed {
            self.selected = None;
            self.flagged.clear();
            self.refresh();
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.depth_view = depth_view(&self.device, w, h);
    }

    fn render(&mut self) {
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

        let aspect = self.config.width as f32 / self.config.height as f32;
        let uniforms = Uniforms {
            view_proj: self.camera.view_proj(aspect).to_cols_array_2d(),
            light: [0.4, -0.6, 0.9, 0.0],
        };
        self.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.93, g: 0.94, b: 0.96, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
    }
}

struct App {
    mesh: Mesh,
    title: String,
    state: Option<State>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title(self.title.clone()))
                .expect("create window"),
        );
        self.state = Some(pollster::block_on(State::new(window, &self.mesh)));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                s.resize(size.width, size.height);
                s.window.request_redraw();
            }
            WindowEvent::RedrawRequested => s.render(),
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        if down {
                            s.press_at = s.cursor;
                        } else {
                            // A release close to the press point is a click, not an orbit.
                            let (dx, dy) = (s.cursor.0 - s.press_at.0, s.cursor.1 - s.press_at.1);
                            if dx * dx + dy * dy < 16.0 {
                                s.pick(s.cursor.0, s.cursor.1);
                            }
                        }
                        s.left_down = down;
                    }
                    MouseButton::Right | MouseButton::Middle => s.pan_down = down,
                    _ => {}
                }
            }
            WindowEvent::ModifiersChanged(m) => s.ctrl = m.state().control_key(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::ArrowUp) => s.push_pull(1.0),
                    Key::Named(NamedKey::ArrowDown) => s.push_pull(-1.0),
                    Key::Named(NamedKey::Escape) => {
                        s.selected = None;
                        s.refresh();
                    }
                    Key::Character(c) if !s.ctrl && c.eq_ignore_ascii_case("i") => s.inspect(),
                    Key::Character(c) if !s.ctrl && c.eq_ignore_ascii_case("f") => s.repair(),
                    Key::Character(c) if s.ctrl && c.eq_ignore_ascii_case("z") => s.history(true),
                    Key::Character(c) if s.ctrl && c.eq_ignore_ascii_case("y") => s.history(false),
                    _ => {}
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = (position.x as f32, position.y as f32);
                let (dx, dy) = (x - s.cursor.0, y - s.cursor.1);
                s.cursor = (x, y);
                if s.left_down {
                    s.camera.orbit(dx, dy);
                    s.window.request_redraw();
                } else if s.pan_down {
                    s.camera.pan(dx, dy);
                    s.window.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                s.camera.zoom(amount);
                s.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    let (mesh, title) = match std::env::args().nth(1) {
        Some(arg) => {
            let path = PathBuf::from(&arg);
            match daedalus_io::import_mesh(&path) {
                Ok(m) => (m, format!("Daedalus - {arg}")),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => (starter_scene(), "Daedalus".to_string()),
    };
    println!("{HELP}");
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App { mesh, title, state: None };
    event_loop.run_app(&mut app).expect("run app");
}
