//! M0 — a 3D Earth in a GORBIE card.
//!
//! A textured-feeling globe (lambert day/night + a procedural lat/lon
//! graticule) rendered by a real wgpu pipeline composited into an egui
//! card via an `egui_wgpu` paint callback. Drag to orbit, scroll to
//! zoom; it idles with a slow spin.
//!
//! This is the first milestone of the spatial-substrate work (see the
//! wiki fragment "Spatial frames, motors, and the TF tree in
//! triblespace"): it validates the 3D-in-egui path — custom wgpu
//! rendering inside a notebook card — with no pile data, no texture
//! asset, and no depth buffer (a convex sphere is rendered correctly by
//! discarding back-facing fragments in the shader).
//!
//! Run it LIVE — the headless capture harness does not execute wgpu
//! paint callbacks, so screenshots of this card come out blank:
//!
//! ```sh
//! cargo run --example globe
//! ```

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use egui_wgpu::wgpu::util::DeviceExt;
use glam::{Mat4, Vec3};
use lissajous::prelude::*;

// ── Camera state (persists across frames) ────────────────────────────

#[derive(Clone)]
struct Camera {
    yaw: f32,
    pitch: f32,
    dist: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.35,
            dist: 3.2,
        }
    }
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    nb.state("globe", Camera::default, |ctx, cam| {
        ctx.section("Globe", |ctx| {
            // The colour-target format egui is rendering into — needed
            // to build a pipeline whose output matches egui's pass.
            // Present only on the wgpu backend (GORBIE selects it live).
            let target_format = lissajous::wgpu_target_format(ctx.ctx());

            let ui = ctx.ui_mut();
            let size = egui::vec2(ui.available_width(), 460.0);
            let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::drag());

            // Orbit on drag.
            if resp.dragged() {
                let d = resp.drag_delta();
                cam.yaw += d.x * 0.008;
                cam.pitch = (cam.pitch - d.y * 0.008).clamp(-1.4, 1.4);
            }
            // Zoom on scroll while hovered.
            if resp.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    cam.dist = (cam.dist * (1.0 - scroll * 0.0015)).clamp(1.6, 12.0);
                }
            }

            // Idle spin + keep animating.
            let t = ui.input(|i| i.time) as f32;
            ui.ctx().request_repaint();
            let yaw = cam.yaw + t * 0.12;

            // Camera → view-projection.
            let aspect = (rect.width() / rect.height()).max(0.01);
            let proj = Mat4::perspective_rh(45f32.to_radians(), aspect, 0.05, 50.0);
            let cp = cam.pitch.cos();
            let eye = Vec3::new(
                cam.dist * cp * yaw.cos(),
                cam.dist * cam.pitch.sin(),
                cam.dist * cp * yaw.sin(),
            );
            let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
            let mvp = proj * view; // model = identity (unit sphere)

            let sun = Vec3::new(0.7, 0.5, 0.45).normalize();
            let uniform = GlobeUniform {
                mvp: mvp.to_cols_array_2d(),
                cam: [eye.x, eye.y, eye.z, 0.0],
                light: [sun.x, sun.y, sun.z, 0.0],
            };

            match target_format {
                Some(format) => {
                    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                        rect,
                        GlobeCallback { uniform, format },
                    ));
                }
                None => {
                    // glow backend / headless — no wgpu callback support.
                    ui.painter().rect_filled(
                        rect,
                        egui::CornerRadius::ZERO,
                        egui::Color32::from_rgb(0x14, 0x1b, 0x2e),
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "globe needs the wgpu backend — run live:\ncargo run --example globe",
                        egui::FontId::monospace(13.0),
                        egui::Color32::from_rgb(0x9a, 0x9a, 0x9a),
                    );
                }
            }
        });
    });
}

// ── GPU uniform ──────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobeUniform {
    mvp: [[f32; 4]; 4],
    cam: [f32; 4],
    light: [f32; 4],
}

// ── Paint callback ───────────────────────────────────────────────────

struct GlobeCallback {
    uniform: GlobeUniform,
    format: wgpu::TextureFormat,
}

impl egui_wgpu::CallbackTrait for GlobeCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Build the GPU resources once, lazily (the device + target
        // format aren't known until the first paint pass).
        if resources.get::<GlobeResources>().is_none() {
            resources.insert(GlobeResources::new(device, self.format));
        }
        let res = resources.get::<GlobeResources>().unwrap();
        queue.write_buffer(&res.uniform_buf, 0, bytemuck::bytes_of(&self.uniform));
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::epaint::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(res) = resources.get::<GlobeResources>() else {
            return;
        };
        render_pass.set_pipeline(&res.pipeline);
        render_pass.set_bind_group(0, &res.bind_group, &[]);
        render_pass.set_vertex_buffer(0, res.vbuf.slice(..));
        render_pass.set_index_buffer(res.ibuf.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..res.index_count, 0, 0..1);
    }
}

// ── GPU resources ────────────────────────────────────────────────────

struct GlobeResources {
    pipeline: wgpu::RenderPipeline,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GlobeResources {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let (verts, indices) = unit_sphere(72, 144);

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe.vbuf"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globe.ibuf"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globe.uniform"),
            size: std::mem::size_of::<GlobeUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globe.bgl"),
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
            label: Some("globe.bg"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("globe.wgsl"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("globe.pl"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("globe.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: (3 * std::mem::size_of::<f32>()) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            // No back-face culling: a convex sphere is rendered correctly
            // without a depth buffer by discarding back-facing fragments
            // in the shader (winding-agnostic, so no inside-out risk).
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            vbuf,
            ibuf,
            index_count: indices.len() as u32,
            uniform_buf,
            bind_group,
        }
    }
}

// ── Geometry ─────────────────────────────────────────────────────────

/// A unit UV sphere. `stacks` rings of latitude, `slices` of longitude.
/// Positions double as normals (radius 1).
fn unit_sphere(stacks: u32, slices: u32) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let theta = std::f32::consts::PI * i as f32 / stacks as f32; // 0..PI
        let (st, ct) = theta.sin_cos();
        for j in 0..=slices {
            let phi = std::f32::consts::TAU * j as f32 / slices as f32; // 0..2PI
            let (sp, cp) = phi.sin_cos();
            verts.push([st * cp, ct, st * sp]);
        }
    }
    let stride = slices + 1;
    let mut indices = Vec::with_capacity((stacks * slices * 6) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * stride + j;
            let b = a + stride;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    (verts, indices)
}

// ── Shader ───────────────────────────────────────────────────────────

const SHADER: &str = r#"
struct U {
  mvp: mat4x4<f32>,
  cam: vec4<f32>,
  light: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;

struct VOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) world: vec3<f32>,
};

@vertex
fn vs(@location(0) pos: vec3<f32>) -> VOut {
  var o: VOut;
  o.clip = u.mvp * vec4<f32>(pos, 1.0);
  o.world = pos;
  return o;
}

// Distance-to-nearest-gridline intensity, antialiased via fwidth.
fn grid(coord: f32, step: f32) -> f32 {
  let g = coord / step;
  let d = abs(fract(g - 0.5) - 0.5) / max(fwidth(g), 1e-5);
  return 1.0 - clamp(d, 0.0, 1.0);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
  let n = normalize(in.world);
  let view_dir = normalize(u.cam.xyz - in.world);
  // Convex-sphere back-face cull without a depth buffer.
  if (dot(n, view_dir) <= 0.0) { discard; }

  let lat = asin(clamp(n.y, -1.0, 1.0));   // -PI/2 .. PI/2
  let lon = atan2(n.z, n.x);               // -PI .. PI
  let step = radians(15.0);
  let line = max(grid(lat, step), grid(lon, step));

  // Lambert day/night.
  let lambert = max(dot(n, normalize(u.light.xyz)), 0.0);
  let shade = 0.18 + 0.82 * lambert;

  let ocean = vec3<f32>(0.10, 0.26, 0.50);
  let grat = vec3<f32>(0.75, 0.85, 1.0);
  let rgb = mix(ocean, grat, line * 0.85) * shade;
  return vec4<f32>(rgb, 1.0);
}
"#;
