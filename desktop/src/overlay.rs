//! L'interface, dessinée par egui dans un contexte GPU sans fenêtre (EGL « surfaceless »), puis
//! confiée à mpv comme superposition (`overlay-add`). Seule la zone réellement dessinée est
//! relue et transmise, et seulement quand l'interface change : interface masquée, coût nul.

use std::sync::Arc;

use egui_glow::glow::{self, HasContext};
use glutin::api::egl::{context::PossiblyCurrentContext, device::Device, display::Display};
use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
use glutin::prelude::*;

use crate::mpv::Mpv;

/// Trois tampons en rotation : mpv lit la superposition au rendu, on n'écrit jamais dans
/// celui qu'il affiche.
const BUFFERS: usize = 3;

pub struct Overlay {
    gl: Arc<glow::Context>,
    _context: PossiblyCurrentContext,
    _display: Display,
    pub painter: egui_glow::Painter,
    target: Option<(glow::Framebuffer, glow::Texture, i32, i32)>,
    buffers: [Vec<u8>; BUFFERS],
    next: usize,
    shown: bool,
    scratch: Vec<u8>,
}

impl Overlay {
    pub fn new() -> Result<Self, String> {
        let device = Device::query_devices().map_err(|e| e.to_string())?.next().ok_or("aucun GPU EGL")?;
        let display = unsafe { Display::with_device(&device, None) }.map_err(|e| e.to_string())?;
        let template = ConfigTemplateBuilder::new().with_surface_type(ConfigSurfaceTypes::empty()).build();
        let config = unsafe { display.find_configs(template) }.map_err(|e| e.to_string())?.next().ok_or("aucune configuration EGL")?;
        let attrs = ContextAttributesBuilder::new().with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3)))).build(None);
        let context = unsafe { display.create_context(&config, &attrs) }.map_err(|e| e.to_string())?;
        let context = context.make_current_surfaceless().map_err(|e| e.to_string())?;
        let gl = Arc::new(unsafe { glow::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) });
        let painter = egui_glow::Painter::new(gl.clone(), "", None, false).map_err(|e| e.to_string())?;
        Ok(Self {
            gl,
            _context: context,
            _display: display,
            painter,
            target: None,
            buffers: Default::default(),
            next: 0,
            shown: false,
            scratch: Vec::new(),
        })
    }

    fn bind_target(&mut self, w: i32, h: i32) {
        let gl = &self.gl;
        unsafe {
            if self.target.map(|t| (t.2, t.3)) != Some((w, h)) {
                if let Some((f, t, _, _)) = self.target.take() {
                    gl.delete_framebuffer(f);
                    gl.delete_texture(t);
                }
                let tex = gl.create_texture().unwrap();
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
                let fbo = gl.create_framebuffer().unwrap();
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
                self.target = Some((fbo, tex, w, h));
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, self.target.map(|t| t.0));
        }
    }

    /// Dessine les primitives et met à jour la superposition de mpv (ou la retire si vide).
    pub fn present(
        &mut self,
        mpv: &Mpv,
        prims: &[egui::ClippedPrimitive],
        textures: &mut egui::TexturesDelta,
        size: [u32; 2],
        ppp: f32,
    ) {
        let (w, h) = (size[0].max(1) as i32, size[1].max(1) as i32);
        // Zone dessinée : union des maillages, bornée par leurs rectangles de découpe.
        let mut bb = egui::Rect::NOTHING;
        for p in prims {
            if let egui::epaint::Primitive::Mesh(m) = &p.primitive {
                let mut r = egui::Rect::NOTHING;
                for v in &m.vertices {
                    r.extend_with(v.pos);
                }
                bb = bb.union(r.intersect(p.clip_rect));
            }
        }
        self.bind_target(w, h);
        unsafe {
            self.gl.viewport(0, 0, w, h);
            self.gl.clear_color(0.0, 0.0, 0.0, 0.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
        self.painter.paint_and_update_textures([w as u32, h as u32], ppp, prims, textures);
        if !bb.is_positive() {
            if self.shown {
                mpv.command(&["overlay-remove", "0"]);
                self.shown = false;
            }
            return;
        }
        let x0 = ((bb.min.x * ppp).floor() as i32 - 2).clamp(0, w);
        let y0 = ((bb.min.y * ppp).floor() as i32 - 2).clamp(0, h);
        let x1 = ((bb.max.x * ppp).ceil() as i32 + 2).clamp(0, w);
        let y1 = ((bb.max.y * ppp).ceil() as i32 + 2).clamp(0, h);
        let (bw, bh) = (x1 - x0, y1 - y0);
        if bw <= 0 || bh <= 0 {
            return;
        }
        let stride = bw as usize * 4;
        self.scratch.resize(stride * bh as usize, 0);
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, self.target.map(|t| t.0));
            self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            // OpenGL compte les lignes depuis le bas : on relit la zone puis on la retourne.
            self.gl.read_pixels(x0, h - y1, bw, bh, glow::BGRA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut self.scratch)));
        }
        let buf = &mut self.buffers[self.next];
        buf.resize(stride * bh as usize, 0);
        for row in 0..bh as usize {
            let src = &self.scratch[(bh as usize - 1 - row) * stride..][..stride];
            buf[row * stride..][..stride].copy_from_slice(src);
        }
        let addr = format!("&{}", buf.as_ptr() as usize);
        mpv.command_sync(&["overlay-add", "0", &x0.to_string(), &y0.to_string(), &addr, "0", "bgra", &bw.to_string(), &bh.to_string(), &stride.to_string()]);
        self.next = (self.next + 1) % BUFFERS;
        self.shown = true;
    }

    pub fn hide(&mut self, mpv: &Mpv) {
        if self.shown {
            mpv.command_sync(&["overlay-remove", "0"]);
            self.shown = false;
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.painter.destroy();
    }
}
