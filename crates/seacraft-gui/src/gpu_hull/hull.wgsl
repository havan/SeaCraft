// The hull on the GPU: triangles lit in the fragment shader, see-through wire lines, and the fullscreen triangle that
// puts the finished view into egui's pass. Everything stays in gamma space, as egui's own shader does.

// Mirrors `ViewUniform`.
struct View {
    clip_from_local: mat4x4<f32>,
    eye: vec4<f32>,
    ortho_to_eye: vec4<f32>,
    light: vec4<f32>,
    wire_colour: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;

struct HullOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec4<f32>,
}

@vertex
fn vs_hull(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec4<f32>,
) -> HullOut {
    var out: HullOut;
    out.clip = view.clip_from_local * vec4<f32>(position, 1.0);
    out.local = position;
    out.normal = normal;
    out.colour = colour;
    return out;
}

// `render::face_shade`: 0.35 to 1.0 by the angle to the light, either side of the face, times 0.82 when the eye is
// behind the face.
@fragment
fn fs_hull(in: HullOut) -> @location(0) vec4<f32> {
    var to_eye = view.ortho_to_eye.xyz;
    if view.eye.w > 0.5 {
        to_eye = normalize(view.eye.xyz - in.local);
    }
    let side = select(0.82, 1.0, dot(in.normal, to_eye) >= 0.0);
    let shade = (0.35 + 0.65 * abs(dot(in.normal, view.light.xyz))) * side;
    return vec4<f32>(in.colour.rgb * shade, 1.0);
}

@vertex
fn vs_wire(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return view.clip_from_local * vec4<f32>(position, 1.0);
}

@fragment
fn fs_wire() -> @location(0) vec4<f32> {
    return view.wire_colour;
}

// The composite has its own bind group layout. Its bindings stay apart from the uniform's, so no two resources of this
// module share a slot.
@group(0) @binding(1) var hull_texture: texture_2d<f32>;
@group(0) @binding(2) var hull_sampler: sampler;

struct CompositeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// One triangle covering the viewport, with texture y running down.
@vertex
fn vs_composite(@builtin(vertex_index) index: u32) -> CompositeOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: CompositeOut;
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_gamma(in: CompositeOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(hull_texture, hull_sampler, in.uv, 0.0);
}

// For an sRGB target, which encodes what it is given: the standard sRGB decoding undoes that, so the bytes come out as
// in `fs_gamma`.
@fragment
fn fs_linear(in: CompositeOut) -> @location(0) vec4<f32> {
    let gamma = textureSampleLevel(hull_texture, hull_sampler, in.uv, 0.0);
    let lower = gamma.rgb / 12.92;
    let higher = pow((gamma.rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return vec4<f32>(select(higher, lower, gamma.rgb < vec3<f32>(0.04045)), gamma.a);
}
