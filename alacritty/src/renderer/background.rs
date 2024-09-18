use std::mem;

use image::open;
use log::warn;

use crate::display::SizeInfo;
use crate::gl;
use crate::gl::types::*;
use crate::renderer::shader::{ShaderProgram, ShaderVersion};
use crate::renderer::{self, cstr};

#[derive(Debug)]
struct BackgroundImage {
    pub path: String,
    pub height: u32,
    pub ratio: f32,
}

#[derive(Debug)]
pub struct BackgroundRenderer {
    // GL buffer objects.
    vao: GLuint,
    u_size_info: GLint,

    program: ShaderProgram,
    vertices: [(f32, f32, f32, f32); 6],
    texture: GLuint,
    background_image: Option<BackgroundImage>,
}

/// Shader sources for rect rendering program.
static BG_SHADER_F: &str = include_str!("../../res/bg.f.glsl");
static BG_SHADER_V: &str = include_str!("../../res/bg.v.glsl");

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

        Ok(Self { vao, program, vertices, texture, u_size_info, background_image: None })
    }

    pub fn should_draw(&self) -> bool {
        self.background_image.is_some()
    }

    pub fn set_background(&mut self, path: &String) {
        if let Some(i) = &self.background_image {
            if &i.path == path {
                return;
            }
        }
        match open(path) {
            Ok(img) => {
                let img = img.into_rgb8();
                self.background_image = Some(BackgroundImage {
                    path: path.clone(),
                    height: img.height(),
                    ratio: img.width() as f32 / img.height() as f32,
                });

                unsafe {
                    gl::BindTexture(gl::TEXTURE_2D, self.texture);
                    gl::TexImage2D(
                        gl::TEXTURE_2D,
                        0,
                        gl::RGB as i32,
                        img.width() as i32,
                        img.height() as i32,
                        0,
                        gl::RGB,
                        gl::UNSIGNED_BYTE,
                        img.as_ptr() as *const _,
                    );
                    gl::BindTexture(gl::TEXTURE_2D, 0);
                }
            },
            Err(e) => {
                warn!("failed to load image ({}): {}", path, e);
                // still set the image so we don't try to load image at every frame
                self.background_image = Some(BackgroundImage {
                    path: path.clone(),
                    height: 0,
                    ratio: 0f32,
                });
            },
        }
    }

    pub fn draw(&self, size: &SizeInfo, alpha: f32) {
        unsafe {
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
        if let Some(img) = &self.background_image {
            unsafe {
                gl::Uniform3f(
                    self.u_size_info,
                    img.ratio * img.height as f32 / size_info.width(),
                    img.height as f32 / size_info.height(),
                    alpha,
                );
            }
        }
    }
}
