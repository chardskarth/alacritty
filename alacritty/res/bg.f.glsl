#if defined(GLES2_RENDERER)
#define FRAG_COLOR gl_FragColor
varying vec2 TexCoords;

#else

out vec4 FragColor;
#define FRAG_COLOR FragColor

in vec2 TexCoords;

#endif

// (width scale, height scale, alpha)
uniform vec3 sizeInfo;
uniform sampler2D bg;
void main() {
  vec4 color = texture(bg, TexCoords);
  color.a = sizeInfo.z;
  FRAG_COLOR = color;
}
