# Helio Standalone Demo

A minimal-but-impressive Helio renderer demo built with the `RendererBuilder`
API. ~200 lines of Rust, zero external assets, fully self-contained.

---

## What it shows

A colonnade with four marble pillars topped by gold spheres, surrounded by
four orbiting coloured point lights, with a glowing blue crystal spinning at
the centre — all running in a free-fly camera viewport.

---

## How it works — line by line

### 1. Project setup (`Cargo.toml`)

```toml
helio = { git = "https://github.com/Far-Beyond-Pulsar/Helio", rev = "1cbd6462" }
helio-default-graphs = { git = "https://github.com/Far-Beyond-Pulsar/Helio", rev = "1cbd6462" }
```

Helio is pulled from the Far-Beyond-Pulsar GitHub repo. Cargo discovers the
workspace in its root `Cargo.toml` and resolves the two member crates
(`helio` = the facade crate, `helio-default-graphs` = the pre-built render
graph). The `rev` pins the exact commit that includes the `RendererBuilder`
changes.

The remaining deps (`wgpu`, `glam`, `winit`, `bytemuck`, `pollster`,
`env_logger`, `log`) are the non-Helio crates the demo uses directly. Helio
pulls its own transitive copies from its own workspace — they don't need to
match.

### 2. The old init path (what we eliminated)

Before the builder, initialising a Helio renderer required ~30 lines of
boilerplate at every call site:

```rust
let scene = Scene::new(device.clone(), queue.clone());

let debug_camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("debug_camera"), size: 64,
    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
});
let cull_stats_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("cull_stats"), size: 64,
    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
});
let debug_state = Arc::new(Mutex::new(DebugDrawState::default()));

let config = RendererConfig::new(w, h, fmt);
let graph = helio_default_graphs::build_default_graph_external(
    &device, &queue, &scene, config, debug_state.clone(),
    &debug_camera_buffer, &cull_stats_buffer, None,
);

let mut renderer = Renderer::new_with_external_device(
    device.clone(), queue.clone(), fmt, w, h, config.render_scale,
    config, scene, graph, debug_state,
    debug_camera_buffer, cull_stats_buffer,
);
renderer.set_editor_mode(true);
renderer.set_ambient([0.0, 0.0, 0.0], 0.0);
renderer.set_clear_color([0.15, 0.18, 0.25, 1.0]);
```

Every caller duplicated this. Every caller had to know about Scene,
DebugDrawState, buffer creation, GraphRebuilder, and the 12-argument
constructor. This was the boilerplate the builder was designed to kill.

### 3. The builder init path

```rust
let mut r = RendererBuilder::new(RendererConfig::new(w, h, fmt))
    .with_editor_mode(true)
    .with_graph(Box::new(|d, q, s, c, ds, cb, csb| {
        build_default_graph_external(d, q, s, c, ds, cb, csb, None)
    }))
    .build(device.clone(), queue.clone(), w, h, fmt);
```

That is the entire init sequence. Four chained calls, no intermediate
variables. Here is what `build()` does internally:

1. Creates `Scene::new(device, queue)` — the caller never touches `Scene`
   directly at init time.
2. Creates a `debug_camera_buffer` (64 bytes, `UNIFORM | COPY_DST`) — no
   caller boilerplate.
3. Creates a `cull_stats_buffer` (64 bytes, `STORAGE | COPY_SRC | COPY_DST`)
   — the `COPY_SRC` flag was the bug this builder walk-through discovered.
4. Creates `Arc::new(Mutex::new(DebugDrawState::default()))`.
5. Calls the closure provided to `with_graph()`, passing borrows of the
   device, queue, scene, config, debug state, and both internal buffers.
   The closure returns a ready-to-use `RenderGraph`.
6. Calls `Renderer::construct()` with all of the above, plus the
   `owns_device` flag (set by `with_external_device()` if the device is
   shared).
7. Applies `set_editor_mode()`, `set_ambient()`, `set_clear_color()` from
   the builder's fields.

The closure pattern is essential: the scene and buffers are created by the
builder, borrowed by the closure to build the graph, then moved into the
Renderer once the closure returns. No dangling references, no clones.

### 4. Scene population

After the builder returns a `Renderer`, the scene is populated through its
handle-based API:

```rust
// Materials — GPU-side PBR params, no textures
let gold   = r.scene_mut().insert_material(make_mat(...));
let marble = r.scene_mut().insert_material(make_mat(...));
let floor  = r.scene_mut().insert_material(make_mat(...));

// Meshes — upload vertex buffers to the GPU, get back a MeshId
let ground = r.scene_mut().insert_actor(SceneActor::mesh(plane_mesh(12.0))).as_mesh().unwrap();

// Objects — bind a mesh + material + transform into a drawable
let _ = add_obj(&mut r, ground, floor, Mat4::IDENTITY, 12.0, None);

// Lights — GPU-side point/directional/spot struct
let light_id = r.scene_mut().insert_actor(SceneActor::light(pt_light(...))).as_light().unwrap();

// Sky — enables the sky-dome pass in the render graph
r.scene_mut().insert_actor(SceneActor::sky(helio::SkyActor::new().with_sky_color([0.15, 0.25, 0.45])));
```

Key insight: `insert_actor` is a unified insertion that dispatches on the
`SceneActor` enum variant. It returns a `SceneActorId` which can be
converted to a concrete handle via `.as_mesh()`, `.as_light()`,
`.as_object()`, etc. These handles are lightweight generational indices —
zero-cost to store and pass around.

The `Movability` parameter on objects controls GPU-side caching:
- `None` (= `Static`) — position never changes, maximum GPU culling
- `Some(Movable)` — position changes every frame, minimal caching

The central crystal uses `Movable` so its per-frame rotation doesn't
invalidate cached culling data.

### 5. Mesh generation

All meshes are built in code using `PackedVertex`. A `PackedVertex` is a
32-byte struct:

```rust
#[repr(C)]
pub struct PackedVertex {
    pub position:       [f32; 3],
    pub bitangent_sign: f32,
    pub tex_coords0:    [f32; 2],
    pub tex_coords1:    [f32; 2],
    pub normal:         u32,   // packed SNORM4x8
    pub tangent:        u32,   // packed SNORM4x8
}
```

The helper `PackedVertex::from_components()` takes unpacked `[f32; 3]`
normal/tangent and packs them internally. The three mesh generators:

- **`cube_mesh(half)`** — 24 verts, 36 indices, 6 faces. The pillar is a
  cube with `position.y *= 13.33` to stretch it into a column.
- **`sphere_mesh(radius)`** — 16×32 lat/lon grid, UV-mapped. Normals are
  exact (computed analytically, not averaged from faces).
- **`plane_mesh(half)`** — 4 verts, 6 indices. Winding is reversed so the
  top face is front-facing (the GPU convention).

### 6. Animation

The render loop runs two animations per frame:

**Orbiting lights** — each of the four point lights traces a circular path
at radius 4, staggered by 90° (`FRAC_PI_2`). Their Y positions also bob
with a sine wave. The light is updated via:

```rust
renderer.scene_mut().update_light(id, pt_light(new_position, color, intensity, range));
```

This is a zero-copy GPU update: the scene marks the light's GPU buffer slot
as dirty, and the next `flush()` uploads only the changed bytes.

**Spinning crystal** — the central cube rotates around Y at 0.8 rad/s and
wobbles around X with a slow sine:

```rust
let rot = Quat::from_rotation_y(t * 0.8) * Quat::from_rotation_x((t * 0.5).sin() * 0.3);
renderer.scene_mut().update_object_transform(id, Mat4::from_rotation_translation(rot, Vec3::new(0.0, 2.5, 0.0)));
```

Because the object was created with `Movability::Movable`, the GPU culling
system knows its bounds change every frame and does not cache them.

### 7. Render loop

```rust
let output = match self.surface.get_current_texture() {
    wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
    _ => return,
};
let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
if let Err(e) = self.renderer.render(&camera, &view) { log::error!("Render: {:?}", e); }
self.queue.present(output);
```

`renderer.render()` takes a `Camera` (computed from the free-fly state) and
a `&wgpu::TextureView` (the swapchain image). It runs the entire render
graph — depth prepass, G-buffer, lighting, sky, post-processing, TAA
upscale — and submits the command buffer to the queue. The caller just calls
`queue.present()`.

---

## What the builder buys you

| Concern | Before (old `new()`) | After (`RendererBuilder`) |
|---------|----------------------|---------------------------|
| Init lines of code | ~30 | ~5 |
| Temporary variables | Scene, 2 buffers, debug_state, config, graph | None |
| Knowledge required | Scene, DebugDrawState, wgpu buffers, GraphRebuilder, 12-param constructor | `RendererBuilder::new(...).with_*(...).build()` |
| Internal buffer sizes | Hardcoded at every call site | Defined once in `builder.rs` |
| `COPY_SRC` flag | Inconsistent across callers | Fixed in one place |

The old constructors (`Renderer::new()`, `Renderer::new_with_external_device()`)
are preserved as `#[deprecated]` wrappers that delegate to the same internal
`construct()` function — zero risk of regressions.

---

## Running

```bash
git clone https://github.com/Far-Beyond-Pulsar/Helio_Standalone_Demo
cd Helio_Standalone_Demo
cargo run --release
```

Controls:

| Input | Action |
|-------|--------|
| WASD | Fly forward/left/back/right |
| Space / Shift | Fly up / down |
| Left click | Grab cursor |
| Mouse drag | Look around |
| Escape | Release cursor (or exit if not grabbed) |
