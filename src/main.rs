//! Helio Standalone Demo — built entirely with the new RendererBuilder.
//!
//! Controls:
//!   WASD        — fly
//!   Space/Shift — up/down
//!   Left click  — grab cursor, drag to look
//!   Escape      — release / exit

use std::collections::HashSet;
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use helio::{
    required_experimental_features, required_wgpu_features, required_wgpu_limits, Camera,
    GpuLight, GpuMaterial, LightId, LightType, MeshUpload, Movability, ObjectDescriptor, ObjectId,
    PackedVertex, RendererBuilder, RendererConfig, SceneActor, GroupMask,
};
use helio_default_graphs::build_default_graph_external;

use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_mat(color: [f32; 4], roughness: f32, metal: f32, emit: [f32; 3], ei: f32) -> GpuMaterial {
    GpuMaterial {
        base_color: color,
        emissive: [emit[0], emit[1], emit[2], ei],
        roughness_metallic: [roughness, metal, 1.5, 0.5],
        tex_base_color: GpuMaterial::NO_TEXTURE,
        tex_normal: GpuMaterial::NO_TEXTURE,
        tex_roughness: GpuMaterial::NO_TEXTURE,
        tex_emissive: GpuMaterial::NO_TEXTURE,
        tex_occlusion: GpuMaterial::NO_TEXTURE,
        workflow: 0,
        flags: 0,
        material_class: 0,
        class_params: [0.0; 4],
    }
}

fn pt_light(pos: [f32; 3], color: [f32; 3], intensity: f32, range: f32) -> GpuLight {
    GpuLight {
        position_range: [pos[0], pos[1], pos[2], range],
        direction_outer: [0.0, 0.0, -1.0, 0.0],
        color_intensity: [color[0], color[1], color[2], intensity],
        shadow_index: 0,
        light_type: LightType::Point as u32,
        inner_angle: 0.0,
        _pad: 0,
        ..Default::default()
    }
}

fn cube_mesh(half: f32) -> MeshUpload {
    use PackedVertex as PV;
    let e = half;
    let c = |x: f32, y: f32, z: f32| [x, y, z];
    // 6 faces × 4 verts, 6 faces × 6 indices
    let (pos, norm, uv, tan): ([[f32; 3]; 24], [[f32; 3]; 24], [[f32; 2]; 24], [[f32; 3]; 24]) = {
        let p = &c;
        let f = |[a, b, cc, d]: [usize; 4], n: [f32; 3], t: [f32; 3]| {
            let u = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
            ([a, b, cc, d], [n; 4], u, [t; 4])
        };
        let (qi, n, u, t): (Vec<[usize; 4]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 3]>) = vec![
            f([0, 1, 2, 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            f([5, 4, 7, 6], [0.0, 0.0, -1.0], [-1.0, 0.0, 0.0]),
            f([4, 0, 3, 7], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            f([1, 5, 6, 2], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            f([3, 2, 6, 7], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
            f([4, 5, 1, 0], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0]),
        ]
        .into_iter()
        .fold(
            (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
            |(mut p, mut n, mut u, mut t), (q, ns, us, ts)| {
                p.push(q); n.extend(ns); u.extend(us); t.extend(ts); (p, n, u, t)
            },
        );
        let cs = [
            [-e, -e, e], [e, -e, e], [e, e, e], [-e, e, e],
            [-e, -e, -e], [e, -e, -e], [e, e, -e], [-e, e, -e],
        ];
        let pts: Vec<[f32; 3]> = qi.iter().flat_map(|q| q.iter().map(|&i| cs[i])).collect();
        (pts.try_into().unwrap(), n.try_into().unwrap(), u.try_into().unwrap(), t.try_into().unwrap())
    };
    let vertices: Vec<PackedVertex> = (0..24)
        .map(|i| PV::from_components(pos[i], norm[i], uv[i], tan[i], 1.0))
        .collect();
    let mut indices = Vec::with_capacity(36);
    for f in 0..6 {
        let b = (f * 4) as u32;
        indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    MeshUpload { vertices, indices }
}

fn sphere_mesh(radius: f32) -> MeshUpload {
    let (lat, lon) = (16, 32);
    let mut verts = Vec::new();
    let mut idx = Vec::new();
    for i in 0..=lat {
        let phi = std::f32::consts::PI * i as f32 / lat as f32;
        let (sp, cp) = phi.sin_cos();
        for j in 0..=lon {
            let theta = 2.0 * std::f32::consts::PI * j as f32 / lon as f32;
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st];
            verts.push(PackedVertex::from_components(
                [n[0] * radius, n[1] * radius, n[2] * radius], n,
                [j as f32 / lon as f32, i as f32 / lat as f32],
                [-n[2], 0.0, n[0]], 1.0,
            ));
        }
    }
    for i in 0..lat {
        for j in 0..lon {
            let a = (i * (lon + 1) + j) as u32;
            let b = a + (lon + 1) as u32;
            idx.extend([a, a + 1, b, b, a + 1, b + 1]);
        }
    }
    MeshUpload { vertices: verts, indices: idx }
}

fn plane_mesh(half: f32) -> MeshUpload {
    let p = |x, z| PackedVertex::from_components([x, 0.0, z], [0.0, 1.0, 0.0], [x / half * 0.5 + 0.5, z / half * 0.5 + 0.5], [1.0, 0.0, 0.0], 1.0);
    MeshUpload {
        vertices: vec![p(-half, -half), p(half, -half), p(half, half), p(-half, half)],
        indices: vec![0, 2, 1, 0, 3, 2],
    }
}

fn add_obj(r: &mut helio::Renderer, mesh: helio::MeshId, mat: helio::MaterialId, t: Mat4, rds: f32, mov: Option<Movability>) -> ObjectId {
    let scene = r.scene_mut();
    let id = scene.insert_actor(SceneActor::object(ObjectDescriptor {
        mesh, material: mat, transform: t,
        bounds: [t.w_axis.x, t.w_axis.y, t.w_axis.z, rds],
        flags: 3, groups: GroupMask::NONE, movability: mov, user_tag: 0,
    }));
    id.as_object().unwrap()
}

// ── App ───────────────────────────────────────────────────────────────────────

fn main() {
    env_logger::init();
    EventLoop::new().unwrap().run_app(&mut App { state: None }).unwrap();
}

struct App { state: Option<State> }

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    fmt: wgpu::TextureFormat,
    renderer: helio::Renderer,
    last_frame: std::time::Instant,
    start_time: std::time::Instant,
    cam_pos: Vec3,
    cam_yaw: f32,
    cam_pitch: f32,
    keys: HashSet<KeyCode>,
    grabbed: bool,
    mouse_delta: (f32, f32),
    lights: [LightId; 4],
    spinner: ObjectId,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.state.is_some() { return }

        let window = Arc::new(el.create_window(Window::default_attributes().with_title("Helio Builder Demo").with_inner_size(winit::dpi::LogicalSize::new(1280, 720))).unwrap());
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), flags: wgpu::InstanceFlags::empty(), ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let surface = instance.create_surface(window.clone()).unwrap();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: Some(&surface), ..Default::default() })).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("D"), required_features: required_wgpu_features(adapter.features()), required_limits: required_wgpu_limits(adapter.limits()), experimental_features: required_experimental_features(adapter.features()), ..Default::default() })).unwrap();
        let device = Arc::new(device); let queue = Arc::new(queue);
        let caps = surface.get_capabilities(&adapter);
        let fmt = caps.formats.iter().find(|f| f.is_srgb()).copied().unwrap_or(caps.formats[0]);
        let (w, h) = (window.inner_size().width.max(1), window.inner_size().height.max(1));
        surface.configure(&device, &wgpu::SurfaceConfiguration { usage: wgpu::TextureUsages::RENDER_ATTACHMENT, format: fmt, width: w, height: h, present_mode: wgpu::PresentMode::Fifo, alpha_mode: caps.alpha_modes[0], view_formats: vec![], desired_maximum_frame_latency: 2, color_space: wgpu::SurfaceColorSpace::Auto });

        // ── Build renderer ──────────────────────────────────────────────────
        let mut r = RendererBuilder::new(RendererConfig::new(w, h, fmt))
            .with_editor_mode(true)
            .with_graph(Box::new(|d, q, s, c, ds, cb, csb| {
                build_default_graph_external(d, q, s, c, ds, cb, csb, None)
            }))
            .build(device.clone(), queue.clone(), w, h, fmt);

        // ── Scene ───────────────────────────────────────────────────────────
        let gold = r.scene_mut().insert_material(make_mat([0.95, 0.75, 0.25, 1.0], 0.25, 0.85, [0.0; 3], 0.0));
        let marble = r.scene_mut().insert_material(make_mat([0.85, 0.83, 0.80, 1.0], 0.55, 0.0, [0.0; 3], 0.0));
        let crystal = r.scene_mut().insert_material(make_mat([0.3, 0.6, 1.0, 1.0], 0.05, 0.1, [0.2, 0.4, 1.0], 2.0));
        let floor = r.scene_mut().insert_material(make_mat([0.22, 0.22, 0.25, 1.0], 0.7, 0.05, [0.0; 3], 0.0));
        r.scene_mut().insert_actor(SceneActor::sky(helio::SkyActor::new().with_sky_color([0.15, 0.25, 0.45])));

        let gnd = r.scene_mut().insert_actor(SceneActor::mesh(plane_mesh(12.0))).as_mesh().unwrap();
        let _ = add_obj(&mut r, gnd, floor, Mat4::IDENTITY, 12.0, None);

        let pillar = r.scene_mut().insert_actor(SceneActor::mesh({
            let mut m = cube_mesh(0.15); m.vertices.iter_mut().for_each(|v| v.position[1] *= 13.33); m // stretch Y
        })).as_mesh().unwrap();
        let sphere = r.scene_mut().insert_actor(SceneActor::mesh(sphere_mesh(0.4))).as_mesh().unwrap();

        let colors = [[1.0, 0.3, 0.3], [0.3, 1.0, 0.3], [0.3, 0.5, 1.0], [1.0, 0.8, 0.2]];
        let mut lights = Vec::new();
        for (i, angle) in [0.0, 90.0f32.to_radians(), 180.0f32.to_radians(), 270.0f32.to_radians()].into_iter().enumerate() {
            let (x, z) = (angle.cos() * 4.0, angle.sin() * 4.0);
            let _ = add_obj(&mut r, pillar, marble, Mat4::from_translation(Vec3::new(x, 2.0, z)), 0.15, None);
            let _ = add_obj(&mut r, sphere, gold, Mat4::from_translation(Vec3::new(x, 4.3, z)), 0.4, None);
            lights.push(r.scene_mut().insert_actor(SceneActor::light(pt_light([x, 5.0, z], colors[i], 8.0, 8.0))).as_light().unwrap());
        }

        let cube = r.scene_mut().insert_actor(SceneActor::mesh(cube_mesh(0.6))).as_mesh().unwrap();
        let spinner = add_obj(&mut r, cube, crystal, Mat4::from_translation(Vec3::new(0.0, 2.5, 0.0)), 0.6, Some(Movability::Movable));

        self.state = Some(State {
            window, surface, device, queue, fmt, renderer: r,
            last_frame: std::time::Instant::now(), start_time: std::time::Instant::now(),
            cam_pos: Vec3::new(0.0, 3.5, 10.0), cam_yaw: 0.0, cam_pitch: -0.25,
            keys: HashSet::new(), grabbed: false, mouse_delta: (0.0, 0.0),
            lights: lights.try_into().unwrap(), spinner,
        });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, e: WindowEvent) {
        let Some(s) = &mut self.state else { return };
        match e {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::KeyboardInput { event: KeyEvent { state: ElementState::Pressed, physical_key: PhysicalKey::Code(KeyCode::Escape), .. }, .. } => {
                if s.grabbed { s.grabbed = false; let _ = s.window.set_cursor_grab(CursorGrabMode::None); s.window.set_cursor_visible(true); }
                else { el.exit(); }
            }
            WindowEvent::KeyboardInput { event: KeyEvent { state: ks, physical_key: PhysicalKey::Code(k), .. }, .. } => match ks {
                ElementState::Pressed => { s.keys.insert(k); }
                ElementState::Released => { s.keys.remove(&k); }
            },
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if !s.grabbed && s.window.set_cursor_grab(CursorGrabMode::Confined).or_else(|_| s.window.set_cursor_grab(CursorGrabMode::Locked)).is_ok() {
                    s.window.set_cursor_visible(false); s.grabbed = true;
                }
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                s.surface.configure(&s.device, &wgpu::SurfaceConfiguration { usage: wgpu::TextureUsages::RENDER_ATTACHMENT, format: s.fmt, width: size.width, height: size.height, present_mode: wgpu::PresentMode::Fifo, alpha_mode: wgpu::CompositeAlphaMode::Auto, view_formats: vec![], desired_maximum_frame_latency: 2, color_space: wgpu::SurfaceColorSpace::Auto });
                s.renderer.set_render_size(size.width, size.height);
            }
            WindowEvent::RedrawRequested => {
                let now = std::time::Instant::now();
                let dt = (now - s.last_frame).as_secs_f32().min(0.05);
                s.last_frame = now; s.render(dt); s.window.request_redraw();
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, e: DeviceEvent) {
        if let Some(s) = &mut self.state {
            if let DeviceEvent::MouseMotion { delta: (dx, dy) } = e { if s.grabbed { s.mouse_delta.0 += dx as f32; s.mouse_delta.1 += dy as f32; } }
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) { if let Some(s) = &self.state { s.window.request_redraw(); } }
}

impl State {
    fn render(&mut self, dt: f32) {
        const S: f32 = 6.0; const SENS: f32 = 0.002;
        self.cam_yaw += self.mouse_delta.0 * SENS;
        self.cam_pitch = (self.cam_pitch - self.mouse_delta.1 * SENS).clamp(-1.5, 1.5);
        self.mouse_delta = (0.0, 0.0);
        let (sy, cy) = self.cam_yaw.sin_cos(); let (sp, cp) = self.cam_pitch.sin_cos();
        let fwd = Vec3::new(sy * cp, sp, -cy * cp); let r = Vec3::new(cy, 0.0, sy);
        if self.keys.contains(&KeyCode::KeyW) { self.cam_pos += fwd * S * dt; }
        if self.keys.contains(&KeyCode::KeyS) { self.cam_pos -= fwd * S * dt; }
        if self.keys.contains(&KeyCode::KeyA) { self.cam_pos -= r * S * dt; }
        if self.keys.contains(&KeyCode::KeyD) { self.cam_pos += r * S * dt; }
        if self.keys.contains(&KeyCode::Space) { self.cam_pos.y += S * dt; }
        if self.keys.contains(&KeyCode::ShiftLeft) { self.cam_pos.y -= S * dt; }

        let size = self.window.inner_size();
        let aspect = size.width as f32 / size.height.max(1) as f32;
        let t = self.start_time.elapsed().as_secs_f32();
        let cam = Camera::perspective_look_at(self.cam_pos, self.cam_pos + fwd, Vec3::Y, std::f32::consts::FRAC_PI_4, aspect, 0.1, 200.0);

        let colors = [[1.0, 0.3, 0.3], [0.3, 1.0, 0.3], [0.3, 0.5, 1.0], [1.0, 0.8, 0.2]];
        for (i, &lid) in self.lights.iter().enumerate() {
            let angle = t * 0.6 + i as f32 * std::f32::consts::FRAC_PI_2;
            let (x, z) = (angle.cos() * 4.0, angle.sin() * 4.0);
            let _ = self.renderer.scene_mut().update_light(lid, pt_light([x, 5.0 + (t * 1.2 + i as f32).sin() * 0.5, z], colors[i], 10.0, 10.0));
        }

        let rot = Quat::from_rotation_y(t * 0.8) * Quat::from_rotation_x((t * 0.5).sin() * 0.3);
        let _ = self.renderer.scene_mut().update_object_transform(self.spinner, Mat4::from_rotation_translation(rot, Vec3::new(0.0, 2.5, 0.0)));

        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return,
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        if let Err(e) = self.renderer.render(&cam, &view) { log::error!("Render: {:?}", e); }
        self.queue.present(output);
    }
}
