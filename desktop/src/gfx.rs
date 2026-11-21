//! Rendu : la vidéo de mpv dans une texture, un flou « dual Kawase » à basse résolution, et le
//! verre liquide — un shader qui réfracte la vidéo floutée sur les bords, la teinte et y pose
//! un reflet spéculaire. Le flou n'est recalculé que lorsqu'une nouvelle image arrive et que du
//! verre est affiché.

use egui_glow::glow::{self, HasContext};

const VS_FULL: &str = r#"#version 330 core
out vec2 uv;
void main() {
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}"#;

const FS_BLIT: &str = r#"#version 330 core
in vec2 uv; out vec4 frag; uniform sampler2D tex;
void main() { frag = vec4(texture(tex, uv).rgb, 1.0); }"#;

const FS_DOWN: &str = r#"#version 330 core
in vec2 uv; out vec4 frag; uniform sampler2D tex; uniform vec2 half_px;
void main() {
    vec4 s = texture(tex, uv) * 4.0;
    s += texture(tex, uv - half_px);
    s += texture(tex, uv + half_px);
    s += texture(tex, uv + vec2(half_px.x, -half_px.y));
    s += texture(tex, uv - vec2(half_px.x, -half_px.y));
    frag = s / 8.0;
}"#;

const FS_UP: &str = r#"#version 330 core
in vec2 uv; out vec4 frag; uniform sampler2D tex; uniform vec2 half_px;
void main() {
    vec2 h = half_px;
    vec4 s = texture(tex, uv + vec2(-h.x * 2.0, 0.0));
    s += texture(tex, uv + vec2(-h.x, h.y)) * 2.0;
    s += texture(tex, uv + vec2(0.0, h.y * 2.0));
    s += texture(tex, uv + vec2(h.x, h.y)) * 2.0;
    s += texture(tex, uv + vec2(h.x * 2.0, 0.0));
    s += texture(tex, uv + vec2(h.x, -h.y)) * 2.0;
    s += texture(tex, uv + vec2(0.0, -h.y * 2.0));
    s += texture(tex, uv + vec2(-h.x, -h.y)) * 2.0;
    frag = s / 12.0;
}"#;

/// Rectangle de verre : sommets calculés depuis le rectangle (marge pour l'ombre portée).
const VS_GLASS: &str = r#"#version 330 core
uniform vec4 rect; uniform vec2 screen; uniform float margin;
out vec2 pos;
void main() {
    vec2 c = vec2(gl_VertexID & 1, (gl_VertexID >> 1) & 1);
    pos = rect.xy - margin + c * (rect.zw + 2.0 * margin);
    vec2 ndc = pos / screen * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}"#;

const FS_GLASS: &str = r#"#version 330 core
in vec2 pos; out vec4 frag;
uniform sampler2D blur; uniform vec2 screen; uniform vec4 rect;
uniform float radius; uniform float tint; uniform float alpha; uniform float has_video;

float sd(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
vec2 grad(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    vec2 s = sign(p);
    if (q.x > 0.0 && q.y > 0.0) return s * normalize(q);
    return q.x > q.y ? vec2(s.x, 0.0) : vec2(0.0, s.y);
}
void main() {
    vec2 half_size = rect.zw * 0.5;
    vec2 p = pos - (rect.xy + half_size);
    float d = sd(p, half_size, radius);
    if (d > 0.0) {
        // Ombre douce sous la plaque.
        float sh = (1.0 - smoothstep(0.0, 22.0, d)) * 0.28 * alpha;
        frag = vec4(0.0, 0.0, 0.0, sh * sh * 2.0);
        return;
    }
    float e = -d;
    vec2 n = grad(p, half_size, radius);
    // Lentille : près du bord, l'image est tirée vers l'extérieur, comme à travers un verre épais.
    float lens = pow(1.0 - smoothstep(0.0, 26.0, e), 2.0);
    vec2 sp = pos - n * lens * 18.0;
    vec3 col = texture(blur, vec2(sp.x / screen.x, 1.0 - sp.y / screen.y)).rgb;
    col = mix(vec3(0.05, 0.06, 0.09), col, has_video);
    // Vibrance : couleurs du dessous un peu ravivées, puis teinte sombre.
    float l = dot(col, vec3(0.299, 0.587, 0.114));
    col = mix(vec3(l), col, 1.35);
    col = mix(col, vec3(0.035, 0.04, 0.055), tint);
    // Reflet vertical et lumière venue d'en haut à gauche.
    float t = (pos.y - rect.y) / rect.w;
    col += vec3(0.11) * (1.0 - t) * (1.0 - t) + 0.015;
    float rim = 1.0 - smoothstep(0.0, 1.8, e);
    float light = 0.25 + 0.75 * max(0.0, dot(n, normalize(vec2(-0.35, -1.0))));
    col += rim * light * 0.65;
    // Liseré intérieur, plus discret en bas.
    float inner = (1.0 - smoothstep(1.8, 4.5, e)) * smoothstep(0.0, 1.8, e);
    col += inner * 0.06 * max(0.0, -n.y + 0.3);
    float aa = smoothstep(0.0, 1.0, e);
    frag = vec4(col, aa * alpha);
}"#;

struct Target {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    w: i32,
    h: i32,
}

pub struct Glass {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    pub tint: f32,
    pub alpha: f32,
}

pub struct Gfx {
    vao: glow::VertexArray,
    blit: glow::Program,
    down: glow::Program,
    up: glow::Program,
    glass: glow::Program,
    video: Option<Target>,
    chain: Vec<Target>,
    blur_dirty: bool,
}

unsafe fn program(gl: &glow::Context, vs: &str, fs: &str) -> glow::Program {
    let p = gl.create_program().unwrap();
    for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
        let s = gl.create_shader(kind).unwrap();
        gl.shader_source(s, src);
        gl.compile_shader(s);
        if !gl.get_shader_compile_status(s) {
            panic!("shader : {}", gl.get_shader_info_log(s));
        }
        gl.attach_shader(p, s);
    }
    gl.link_program(p);
    if !gl.get_program_link_status(p) {
        panic!("programme : {}", gl.get_program_info_log(p));
    }
    p
}

unsafe fn target(gl: &glow::Context, w: i32, h: i32, format: u32) -> Target {
    let tex = gl.create_texture().unwrap();
    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
    gl.tex_image_2d(glow::TEXTURE_2D, 0, format as i32, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
    for (k, v) in [
        (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
        (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
    ] {
        gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
    }
    let fbo = gl.create_framebuffer().unwrap();
    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
    gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
    Target { fbo, tex, w, h }
}

unsafe fn drop_target(gl: &glow::Context, t: Target) {
    gl.delete_framebuffer(t.fbo);
    gl.delete_texture(t.tex);
}

impl Gfx {
    pub fn new(gl: &glow::Context) -> Self {
        unsafe {
            Self {
                vao: gl.create_vertex_array().unwrap(),
                blit: program(gl, VS_FULL, FS_BLIT),
                down: program(gl, VS_FULL, FS_DOWN),
                up: program(gl, VS_FULL, FS_UP),
                glass: program(gl, VS_GLASS, FS_GLASS),
                video: None,
                chain: Vec::new(),
                blur_dirty: true,
            }
        }
    }

    /// Cible où mpv dessine (taille de la fenêtre) ; recréée au redimensionnement.
    pub fn video_fbo(&mut self, gl: &glow::Context, w: i32, h: i32) -> u32 {
        unsafe {
            if self.video.as_ref().map(|t| (t.w, t.h)) != Some((w, h)) {
                if let Some(t) = self.video.take() {
                    drop_target(gl, t);
                }
                for t in self.chain.drain(..) {
                    drop_target(gl, t);
                }
                self.video = Some(target(gl, w, h, glow::RGBA8));
                // 1/2, 1/4, 1/8, 1/16 : quatre descentes, deux remontées → flou large et bon marché.
                for i in 1..=4 {
                    self.chain.push(target(gl, (w >> i).max(1), (h >> i).max(1), glow::RGBA8));
                }
            }
            self.blur_dirty = true;
            self.video.as_ref().unwrap().fbo.0.get()
        }
    }

    pub fn mark_frame(&mut self) {
        self.blur_dirty = true;
    }

    unsafe fn pass(&self, gl: &glow::Context, prog: glow::Program, src: glow::Texture, src_w: i32, src_h: i32, dst: &Target) {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(dst.fbo));
        gl.viewport(0, 0, dst.w, dst.h);
        gl.use_program(Some(prog));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(src));
        gl.uniform_1_i32(gl.get_uniform_location(prog, "tex").as_ref(), 0);
        gl.uniform_2_f32(gl.get_uniform_location(prog, "half_px").as_ref(), 1.5 / src_w as f32, 1.5 / src_h as f32);
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
    }

    /// Dessine la vidéo à l'écran, puis le verre par-dessus.
    pub fn compose(&mut self, gl: &glow::Context, w: i32, h: i32, has_video: bool, glass: &[Glass]) {
        unsafe {
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.vao));
            if has_video && !glass.is_empty() && self.blur_dirty && self.video.is_some() {
                let v = self.video.as_ref().unwrap();
                self.pass(gl, self.down, v.tex, v.w, v.h, &self.chain[0]);
                for i in 1..4 {
                    self.pass(gl, self.down, self.chain[i - 1].tex, self.chain[i - 1].w, self.chain[i - 1].h, &self.chain[i]);
                }
                self.pass(gl, self.up, self.chain[3].tex, self.chain[3].w, self.chain[3].h, &self.chain[2]);
                self.pass(gl, self.up, self.chain[2].tex, self.chain[2].w, self.chain[2].h, &self.chain[1]);
                self.blur_dirty = false;
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, w, h);
            if has_video && self.video.is_some() {
                gl.use_program(Some(self.blit));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(self.video.as_ref().unwrap().tex));
                gl.uniform_1_i32(gl.get_uniform_location(self.blit, "tex").as_ref(), 0);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            } else {
                gl.clear_color(0.02, 0.027, 0.047, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            if glass.is_empty() {
                return;
            }
            gl.enable(glow::BLEND);
            gl.blend_func_separate(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            let p = self.glass;
            gl.use_program(Some(p));
            gl.active_texture(glow::TEXTURE0);
            if let Some(t) = self.chain.get(1) {
                gl.bind_texture(glow::TEXTURE_2D, Some(t.tex));
            }
            let u = |n: &str| gl.get_uniform_location(p, n);
            gl.uniform_1_i32(u("blur").as_ref(), 0);
            gl.uniform_2_f32(u("screen").as_ref(), w as f32, h as f32);
            gl.uniform_1_f32(u("margin").as_ref(), 24.0);
            gl.uniform_1_f32(u("has_video").as_ref(), if has_video { 1.0 } else { 0.0 });
            for g in glass {
                gl.uniform_4_f32(u("rect").as_ref(), g.x, g.y, g.w, g.h);
                gl.uniform_1_f32(u("radius").as_ref(), g.radius.min(g.h / 2.0).min(g.w / 2.0));
                gl.uniform_1_f32(u("tint").as_ref(), g.tint);
                gl.uniform_1_f32(u("alpha").as_ref(), g.alpha);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }
            gl.disable(glow::BLEND);
        }
    }
}
