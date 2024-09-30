mod video;

use std::fmt::Debug;
use std::mem;
use video::BackgroundVideo;
use winit::window::WindowId;

use crate::display::SizeInfo;
use crate::gl::types::*;
use crate::renderer::shader::{ShaderProgram, ShaderVersion};
use crate::renderer::{self, cstr};
use crate::scheduler::Scheduler;
use crate::gl;

pub struct BackgroundRenderer {
    // GL buffer objects.
    vao: GLuint,
    u_size_info: GLint,

    program: ShaderProgram,
    vertices: [(f32, f32, f32, f32); 6],
    texture: GLuint,
    background_video: Option<BackgroundVideo>,
}

/// Shader sources for rect rendering program.
static BG_SHADER_F: &str = include_str!("../../../res/bg.f.glsl");
static BG_SHADER_V: &str = include_str!("../../../res/bg.v.glsl");

impl Debug for BackgroundRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundRenderer")
            .field("vao", &self.vao)
            .field("u_size_info", &self.u_size_info)
            .field("program", &self.program)
            .field("vertices", &self.vertices)
            .field("texture", &self.texture)
            .finish()
    }
}

impl BackgroundRenderer {
    pub fn new(shader_version: ShaderVersion) -> Result<Self, renderer::Error> {
        let mut vao: GLuint = 0;
        let mut vbo: GLuint = 0;
        let vertices = [
            (-1f32, 1f32, 0f32, 0f32),
            (1f32, 1f32, 1.0, 0f32),
            (1f32, -1f32, 1.0, 1.0),
            (1f32, -1f32, 1.0, 1.0),
            (-1f32, -1f32, 0f32, 1.0),
            (-1f32, 1f32, 0f32, 0f32),
        ];

        let program = ShaderProgram::new(shader_version, None, BG_SHADER_V, BG_SHADER_F)?;
        let u_size_info = program.get_uniform_location(cstr!("sizeInfo"))?;
        let mut texture: GLuint = 0;
        unsafe {
            // Allocate buffers.
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);

            gl::BindVertexArray(vao);

            // VBO binding is not part of VAO itself, but VBO binding is stored in attributes.
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * mem::size_of::<(f32, f32, f32, f32)>()) as isize,
                vertices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            // Position.
            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                mem::size_of::<(f32, f32, f32, f32)>() as i32,
                0i32 as *const _,
            );
            gl::EnableVertexAttribArray(0);
            // TexCoord
            gl::VertexAttribPointer(
                1,
                2,
                gl::FLOAT,
                gl::FALSE,
                mem::size_of::<(f32, f32, f32, f32)>() as i32,
                (mem::size_of::<f32>() * 2) as *const _,
            );
            gl::EnableVertexAttribArray(1);

            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::REPEAT as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::REPEAT as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);

            // Reset buffer bindings.
            gl::BindVertexArray(0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }

        Ok(Self { vao, program, vertices, texture, u_size_info, background_video: None })
    }

    pub fn should_draw(&self) -> bool {
        self.background_video.is_some()
    }

    pub fn set_background(
        &mut self,
        path: &String,
        scheduler: &mut Scheduler,
        window_id: WindowId,
    ) {
        if self.background_video.is_some() {
            return;
        }

        if let Ok(v) = BackgroundVideo::new(path, scheduler, window_id) {
            self.background_video = Some(v);
        }
    }

    pub fn draw(&mut self, size: &SizeInfo, alpha: f32) {
        if let Some(decoder) = &mut self.background_video {
            decoder.update_frame(self.texture);
        }

        unsafe {
            gl::ClearColor(0f32, 0f32, 0f32, alpha);
            gl::Clear(gl::COLOR_BUFFER_BIT);
            gl::BlendFuncSeparate(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::SRC_ALPHA, gl::ONE);
            // Bind VAO to enable vertex attribute slots.
            gl::BindVertexArray(self.vao);
            gl::UseProgram(self.program.id());
            gl::BindTexture(gl::TEXTURE_2D, self.texture);
        }

        self.update_uniforms(size, alpha);

        unsafe {
            // Draw all vertices as list of triangles.
            gl::DrawArrays(gl::TRIANGLES, 0, self.vertices.len() as i32);

            // Disable program.
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
            // Reset blending strategy.
            gl::BlendFunc(gl::SRC1_COLOR, gl::ONE_MINUS_SRC1_COLOR);
        }
    }

    pub fn update_uniforms(&self, size_info: &SizeInfo, alpha: f32) {
        if let Some(decoder) = &self.background_video {
            let mut width_scale = decoder.width as f32 / size_info.width();
            let mut height_scale = decoder.height as f32 / size_info.height();
            if width_scale < 1f32 {
                width_scale = 1f32;
                height_scale *= decoder.ratio;
            }
            unsafe {
                gl::Uniform3f(self.u_size_info, width_scale, height_scale, alpha);
            }
        }
    }
}
