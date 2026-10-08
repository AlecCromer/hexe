//! wgpu viewport renderer: flat-shaded triangles (headlight), edges/wireframe, round points, grid + axes.
//! One `Scene3d` draws into any colour+depth target. The egui window renders it into an offscreen texture inside an
//! `egui_wgpu` paint callback and blits that into the viewport rect; `render_offscreen` is the same code headless.
use crate::viewport::{Camera, Opts, Style};
use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::{self, wgpu};
use glam::{Mat4, Vec3};
use hexe_geo::{Detail, PrimKind};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use wgpu::util::DeviceExt;

/// Offscreen colour format (gamma-space colours; blit converts for sRGB surfaces).
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Draw caps (a huge cook must never freeze the UI or exhaust GPU memory).
pub const MAX_TRIS: usize = 1_000_000;
pub const MAX_EDGES: usize = 2_000_000;
pub const MAX_POINTS: usize = 500_000;
/// Largest offscreen target edge (pixels).
pub const MAX_TARGET: u32 = 8192;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub col: [f32; 3],
}

static MESH_ID: AtomicU64 = AtomicU64::new(1);

/// CPU-side draw data for one cooked `Detail`. `id` is unique per build so the GPU copy is re-uploaded only on change.
pub struct Mesh {
    pub id: u64,
    pub tris: Vec<Vertex>,
    pub edges: Vec<LineVertex>,
    pub points: Vec<[f32; 3]>,
}
impl Mesh {
    /// Fan-triangulate closed polys (flat Houdini-rule normal per face); edges from polys + polylines; every point.
    pub fn build(d: &Detail, style: &Style) -> Mesh {
        let (mut tris, mut edges) = (Vec::new(), Vec::new());
        let p = d.p();
        for prim in &d.prims {
            let closed = match prim.kind {
                PrimKind::Poly { closed } => closed,
                PrimKind::Polyline => false,
                _ => continue,
            };
            let ids = d.vertex_point.get(prim.verts.start as usize..prim.verts.end as usize).unwrap_or(&[]);
            let w: Vec<[f32; 3]> = ids.iter().filter_map(|&i| p.get(i as usize).copied()).collect();
            if w.len() != ids.len() || w.len() < 2 {
                continue;
            }
            if edges.len() < MAX_EDGES {
                let n = if closed { w.len() } else { w.len() - 1 };
                for i in 0..n {
                    let (a, b) = (w[i], w[(i + 1) % w.len()]);
                    edges.push(LineVertex { pos: a, col: style.edge });
                    edges.push(LineVertex { pos: b, col: style.edge });
                }
            }
            if closed && w.len() >= 3 && tris.len() + 3 * (w.len() - 2) <= MAX_TRIS * 3 {
                let mut n = [0.0f32; 3];
                for (i, a) in w.iter().enumerate() {
                    let b = w[(i + 1) % w.len()];
                    n[0] += (a[1] - b[1]) * (a[2] + b[2]);
                    n[1] += (a[2] - b[2]) * (a[0] + b[0]);
                    n[2] += (a[0] - b[0]) * (a[1] + b[1]);
                }
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                // Houdini's rule (H32): clockwise from the front = front-facing, i.e. the NEGATED right-handed Newell sum
                let nrm = if l > 1e-20 { [-n[0] / l, -n[1] / l, -n[2] / l] } else { [0.0, 1.0, 0.0] };
                for k in 1..w.len() - 1 {
                    for q in [w[0], w[k], w[k + 1]] {
                        tris.push(Vertex { pos: q, nrm });
                    }
                }
            }
        }
        let points = p.iter().copied().take(MAX_POINTS).collect();
        Mesh { id: MESH_ID.fetch_add(1, Ordering::Relaxed), tris, edges, points }
    }
}

/// Per-frame inputs.
#[derive(Clone, Copy)]
pub struct Frame {
    pub view_proj: Mat4,
    pub eye: Vec3,
    pub size_px: [u32; 2],
    pub wire: bool,
}
impl Frame {
    pub fn new(cam: &Camera, style: &Style, opts: &Opts, size_px: [u32; 2]) -> Frame {
        let aspect = size_px[0].max(1) as f32 / size_px[1].max(1) as f32;
        let (view_proj, eye) = cam.matrices(style, aspect);
        Frame { view_proj, eye, size_px, wire: opts.wire }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    /// viewport w, h, point radius px, _
    view: [f32; 4],
    base: [f32; 4],
    point: [f32; 4],
    /// ambient, diffuse, point depth bias, _
    light: [f32; 4],
}
fn rgba(c: [f32; 3]) -> [f32; 4] {
    [c[0], c[1], c[2], 1.0]
}

const SHADER: &str = r#"
struct U { view_proj: mat4x4<f32>, eye: vec4<f32>, view: vec4<f32>, base: vec4<f32>, point: vec4<f32>, light: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
struct TriOut { @builtin(position) pos: vec4<f32>, @location(0) wpos: vec3<f32>, @location(1) nrm: vec3<f32> };
@vertex fn vs_tri(@location(0) p: vec3<f32>, @location(1) n: vec3<f32>) -> TriOut {
    var o: TriOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    o.wpos = p;
    o.nrm = n;
    return o;
}
@fragment fn fs_tri(i: TriOut) -> @location(0) vec4<f32> {
    let v = normalize(u.eye.xyz - i.wpos);
    let d = abs(dot(normalize(i.nrm), v));
    return vec4<f32>(u.base.rgb * (u.light.x + u.light.y * d), 1.0);
}
struct LineOut { @builtin(position) pos: vec4<f32>, @location(0) col: vec3<f32> };
@vertex fn vs_line(@location(0) p: vec3<f32>, @location(1) c: vec3<f32>) -> LineOut {
    var o: LineOut;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    o.col = c;
    return o;
}
@fragment fn fs_line(i: LineOut) -> @location(0) vec4<f32> { return vec4<f32>(i.col, 1.0); }
struct PtOut { @builtin(position) pos: vec4<f32>, @location(0) c: vec2<f32> };
@vertex fn vs_pt(@builtin(vertex_index) vi: u32, @location(0) p: vec3<f32>) -> PtOut {
    var corner = array<vec2<f32>, 4>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0));
    var o: PtOut;
    var clip = u.view_proj * vec4<f32>(p, 1.0);
    let c = corner[vi];
    clip.z = clip.z - u.light.z * clip.w;
    o.pos = vec4<f32>(clip.xy + c * (2.0 * u.view.z) / u.view.xy * clip.w, clip.z, clip.w);
    o.c = c;
    return o;
}
@fragment fn fs_pt(i: PtOut) -> @location(0) vec4<f32> {
    if (dot(i.c, i.c) > 1.0) { discard; }
    return vec4<f32>(u.point.rgb, 1.0);
}
"#;

const BLIT: &str = r#"
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
struct O { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) vi: u32) -> O {
    var c = array<vec2<f32>, 4>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0));
    var o: O;
    o.pos = vec4<f32>(c[vi].x * 2.0 - 1.0, 1.0 - c[vi].y * 2.0, 0.0, 1.0);
    o.uv = c[vi];
    return o;
}
@fragment fn fs_gamma(i: O) -> @location(0) vec4<f32> { return textureSample(t, s, i.uv); }
@fragment fn fs_linear(i: O) -> @location(0) vec4<f32> {
    let c = textureSample(t, s, i.uv);
    return vec4<f32>(pow(c.rgb, vec3<f32>(2.2)), 1.0);
}
"#;

struct MeshBuffers {
    id: u64,
    tris: Option<(wgpu::Buffer, u32)>,
    edges: Option<(wgpu::Buffer, u32)>,
    points: Option<(wgpu::Buffer, u32)>,
}
fn upload<T: Pod>(device: &wgpu::Device, v: &[T], usage: wgpu::BufferUsages) -> Option<(wgpu::Buffer, u32)> {
    (!v.is_empty()).then(|| {
        let b = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hexe mesh"),
            contents: bytemuck::cast_slice(v),
            usage,
        });
        (b, v.len() as u32)
    })
}

pub struct Scene3d {
    tri: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    pt: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    grid: Option<(wgpu::Buffer, u32)>,
    mesh: Option<MeshBuffers>,
    style: Style,
}
impl Scene3d {
    pub fn new(device: &wgpu::Device, style: Style) -> Scene3d {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hexe viewport"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hexe viewport bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hexe viewport layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hexe viewport uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hexe viewport bind"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let pattrs = wgpu::vertex_attr_array![0 => Float32x3];
        let mk = |label: &str,
                  vs: &str,
                  fs: &str,
                  buf: wgpu::VertexBufferLayout<'_>,
                  topology: wgpu::PrimitiveTopology,
                  cmp: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(buf)],
                },
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(cmp),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(COLOR_FORMAT.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let vlayout = |stride: usize, step_mode, attributes| wgpu::VertexBufferLayout {
            array_stride: stride as u64,
            step_mode,
            attributes,
        };
        let vertex = wgpu::VertexStepMode::Vertex;
        let tri = mk(
            "hexe tris",
            "vs_tri",
            "fs_tri",
            vlayout(std::mem::size_of::<Vertex>(), vertex, &attrs),
            wgpu::PrimitiveTopology::TriangleList,
            wgpu::CompareFunction::Less,
        );
        let line = mk(
            "hexe lines",
            "vs_line",
            "fs_line",
            vlayout(std::mem::size_of::<LineVertex>(), vertex, &attrs),
            wgpu::PrimitiveTopology::LineList,
            wgpu::CompareFunction::LessEqual,
        );
        let pt = mk(
            "hexe points",
            "vs_pt",
            "fs_pt",
            vlayout(12, wgpu::VertexStepMode::Instance, &pattrs),
            wgpu::PrimitiveTopology::TriangleStrip,
            wgpu::CompareFunction::LessEqual,
        );
        let grid = upload(device, &grid_lines(&style), wgpu::BufferUsages::VERTEX);
        Scene3d { tri, line, pt, ubuf, bind, grid, mesh: None, style }
    }
    /// Re-upload only when the mesh changed (`id`).
    pub fn set_mesh(&mut self, device: &wgpu::Device, mesh: Option<&Mesh>) {
        let id = mesh.map_or(0, |m| m.id);
        if self.mesh.as_ref().map_or(0, |m| m.id) == id {
            return;
        }
        self.mesh = mesh.map(|m| MeshBuffers {
            id: m.id,
            tris: upload(device, &m.tris, wgpu::BufferUsages::VERTEX),
            edges: upload(device, &m.edges, wgpu::BufferUsages::VERTEX),
            points: upload(device, &m.points, wgpu::BufferUsages::VERTEX),
        });
    }
    /// Record one pass: clear, grid+axes, tris (shaded) or edges (wire), points.
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        f: &Frame,
    ) {
        let s = &self.style;
        let u = Uniforms {
            view_proj: f.view_proj.to_cols_array_2d(),
            eye: [f.eye.x, f.eye.y, f.eye.z, 1.0],
            view: [f.size_px[0].max(1) as f32, f.size_px[1].max(1) as f32, s.point_px, 0.0],
            base: rgba(s.surface),
            point: rgba(s.point),
            light: [s.ambient, s.diffuse, s.point_depth_bias, 0.0],
        };
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
        let bg = s.background;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hexe viewport"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(bg[0]),
                        g: f64::from(bg[1]),
                        b: f64::from(bg[2]),
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_pipeline(&self.line);
        if let Some((b, n)) = &self.grid {
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..*n, 0..1);
        }
        let Some(m) = &self.mesh else { return };
        if !f.wire
            && let Some((b, n)) = &m.tris
        {
            pass.set_pipeline(&self.tri);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..*n, 0..1);
        }
        // Shaded draws surfaces only (open polylines show in wireframe); wire draws every edge.
        if f.wire
            && let Some((b, n)) = &m.edges
        {
            pass.set_pipeline(&self.line);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..*n, 0..1);
        }
        if let Some((b, n)) = &m.points {
            pass.set_pipeline(&self.pt);
            pass.set_vertex_buffer(0, b.slice(..));
            pass.draw(0..4, 0..*n);
        }
    }
}

/// Ground grid on y = 0 (the two axis lines are coloured X red / Z blue) plus a Y axis.
pub fn grid_lines(s: &Style) -> Vec<LineVertex> {
    let mut v = Vec::new();
    let h = s.grid_half as i32;
    let ext = s.grid_half as f32 * s.grid_step;
    for i in -h..=h {
        let o = i as f32 * s.grid_step;
        let c = if i == 0 { s.axis_x } else { s.grid };
        v.push(LineVertex { pos: [-ext, 0.0, o], col: c });
        v.push(LineVertex { pos: [ext, 0.0, o], col: c });
        let c = if i == 0 { s.axis_z } else { s.grid };
        v.push(LineVertex { pos: [o, 0.0, -ext], col: c });
        v.push(LineVertex { pos: [o, 0.0, ext], col: c });
    }
    v.push(LineVertex { pos: [0.0; 3], col: s.axis_y });
    v.push(LineVertex { pos: [0.0, ext, 0.0], col: s.axis_y });
    v
}

/// Colour + depth target of a given size.
pub struct Target {
    pub size: [u32; 2],
    pub color: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    pub depth_view: wgpu::TextureView,
}
impl Target {
    pub fn new(device: &wgpu::Device, size: [u32; 2]) -> Target {
        let size = [size[0].clamp(1, MAX_TARGET), size[1].clamp(1, MAX_TARGET)];
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let tex = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hexe viewport target"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = tex(
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = tex(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let color_view = color.create_view(&Default::default());
        let depth_view = depth.create_view(&Default::default());
        Target { size, color, color_view, depth_view }
    }
}

/// Callback resource inside the egui renderer: scene + blit pipeline + the current offscreen target.
pub struct Renderer3d {
    scene: Scene3d,
    blit: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target: Option<(Target, wgpu::BindGroup)>,
}
impl Renderer3d {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, style: Style) -> Renderer3d {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hexe blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hexe blit bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hexe blit layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hexe blit"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(if target_format.is_srgb() { "fs_linear" } else { "fs_gamma" }),
                compilation_options: Default::default(),
                targets: &[Some(target_format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Renderer3d { scene: Scene3d::new(device, style), blit, bgl, sampler, target: None }
    }
    fn ensure_target(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let size = [size[0].clamp(1, MAX_TARGET), size[1].clamp(1, MAX_TARGET)];
        if self.target.as_ref().is_some_and(|(t, _)| t.size == size) {
            return;
        }
        let t = Target::new(device, size);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hexe blit bind"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&t.color_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.target = Some((t, bind));
    }
}

/// The egui paint callback: render the scene offscreen in `prepare`, blit into the rect in `paint`.
pub struct ViewCallback {
    pub mesh: Option<Arc<Mesh>>,
    pub frame: Frame,
}
impl egui_wgpu::CallbackTrait for ViewCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        res: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = res.get_mut::<Renderer3d>() else { return Vec::new() };
        r.scene.set_mesh(device, self.mesh.as_deref());
        r.ensure_target(device, self.frame.size_px);
        if let Some((t, _)) = &r.target {
            r.scene.draw(queue, encoder, &t.color_view, &t.depth_view, &self.frame);
        }
        Vec::new()
    }
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        res: &egui_wgpu::CallbackResources,
    ) {
        let Some(r) = res.get::<Renderer3d>() else { return };
        let Some((_, bind)) = &r.target else { return };
        pass.set_pipeline(&r.blit);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..4, 0..1);
    }
}

/// Tightly-packed RGBA8 image.
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}
impl Image {
    #[cfg(test)]
    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y.min(self.h.saturating_sub(1)) * self.w + x.min(self.w.saturating_sub(1))) * 4) as usize;
        self.rgba.get(i..i + 4).map_or([0; 4], |s| [s[0], s[1], s[2], s[3]])
    }
    pub fn write_png(&self, path: &str) -> Result<(), String> {
        let f = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), self.w, self.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| format!("{path}: {e}"))?;
        w.write_image_data(&self.rgba).map_err(|e| format!("{path}: {e}"))
    }
}

/// Headless render through the exact same `Scene3d` the window uses. `Err` when no GPU adapter exists.
pub fn render_offscreen(
    mesh: Option<&Mesh>,
    cam: &Camera,
    opts: &Opts,
    style: &Style,
    size: [u32; 2],
) -> Result<Image, String> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .map_err(|e| format!("no wgpu adapter: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .map_err(|e| format!("wgpu device: {e}"))?;
    let mut scene = Scene3d::new(&device, style.clone());
    scene.set_mesh(&device, mesh);
    let target = Target::new(&device, size);
    let [w, h] = target.size;
    let frame = Frame::new(cam, style, opts, target.size);
    let mut enc = device.create_command_encoder(&Default::default());
    scene.draw(&queue, &mut enc, &target.color_view, &target.depth_view, &frame);
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hexe readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        target.color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("wgpu poll: {e}"))?;
    rx.recv().map_err(|e| format!("readback: {e}"))?.map_err(|e| format!("readback: {e}"))?;
    let data = buf.slice(..).get_mapped_range().map_err(|e| format!("readback: {e}"))?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let o = y * row as usize;
        rgba.extend_from_slice(data.get(o..o + (w * 4) as usize).ok_or("readback: short buffer")?);
    }
    Ok(Image { w, h, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail() -> Detail {
        let mut d = Detail::default();
        let r = d.add_points(6);
        for (i, p) in
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [2.0, 0.0, 0.0], [3.0, 1.0, 0.0]]
                .into_iter()
                .enumerate()
        {
            d.p_mut()[i] = p;
        }
        let ids: Vec<u32> = r.collect();
        d.add_poly(&ids[0..4], true);
        d.add_poly(&[ids[1], ids[4], ids[5]], false);
        d
    }

    #[test]
    fn mesh_fan_triangulates_with_flat_normals_and_edges_points() {
        let m = Mesh::build(&detail(), &Style::default());
        assert_eq!(m.tris.len(), 6, "quad -> 2 tris, open polyline -> none");
        assert!(m.tris.iter().all(|v| v.nrm == [0.0, 0.0, 1.0] || v.nrm == [0.0, 0.0, -1.0]), "{:?}", m.tris[0].nrm);
        assert_eq!(m.edges.len(), 2 * (4 + 2), "closed quad 4 + open 3-pt line 2 segments");
        assert_eq!(m.points.len(), 6);
        assert_ne!(Mesh::build(&detail(), &Style::default()).id, m.id, "fresh id per build");
    }

    #[test]
    fn degenerate_faces_never_produce_nan_normals() {
        let mut d = Detail::default();
        let r = d.add_points(3);
        let ids: Vec<u32> = r.collect();
        d.add_poly(&ids, true); // three coincident points
        d.add_poly(&[0, 99, 1], true); // out-of-range id: dropped by the loader
        let m = Mesh::build(&d, &Style::default());
        assert!(m.tris.iter().all(|v| v.nrm.iter().all(|c| c.is_finite())));
    }

    #[test]
    fn grid_has_axes_and_respects_params() {
        let s = Style { grid_half: 2, grid_step: 0.5, ..Style::default() };
        let g = grid_lines(&s);
        assert_eq!(g.len(), 2 * 2 * 5 + 2, "5 lines per direction + Y axis");
        assert!(
            g.iter().any(|v| v.col == s.axis_x)
                && g.iter().any(|v| v.col == s.axis_z)
                && g.iter().any(|v| v.col == s.axis_y)
        );
        assert!(g.iter().all(|v| v.pos.iter().all(|c| c.abs() <= 1.0 + 1e-6)), "extent = half * step");
    }
}
