#if defined(GLES2_RENDERER)
attribute vec2 aPos;
attribute vec2 texCoord;
#else
layout (location = 0) in vec2 aPos;
layout (location = 1) in vec2 texCoord;
#endif

out vec2 TexCoords;
// (width scale, height scale, alpha)
uniform vec3 sizeInfo;

void main() {
    gl_Position = vec4(aPos.x * sizeInfo.x, aPos.y * sizeInfo.y, 0.0, 1.0);
    TexCoords = vec2(texCoord.x, texCoord.y);
}
