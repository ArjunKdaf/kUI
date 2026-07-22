// kUI CRT: scanlines, aperture tint, mild vignette. Single pass.
#if defined(VERTEX)
attribute vec4 VertexCoord;
attribute vec4 TexCoord;
uniform mat4 MVPMatrix;
varying vec2 v_tex;
void main() {
    gl_Position = MVPMatrix * VertexCoord;
    v_tex = TexCoord.xy;
}
#elif defined(FRAGMENT)
uniform sampler2D Texture;
uniform vec2 TextureSize;
uniform vec2 OutputSize;
varying vec2 v_tex;
void main() {
    vec3 col = texture2D(Texture, v_tex).rgb;
    // scanlines: darken between source rows
    float line = sin(v_tex.y * TextureSize.y * 3.14159 * 2.0);
    col *= 0.82 + 0.18 * (line * 0.5 + 0.5);
    // aperture: alternate rgb emphasis by output column
    float px = mod(gl_FragCoord.x, 3.0);
    vec3 mask = px < 1.0 ? vec3(1.05, 0.95, 0.95)
              : px < 2.0 ? vec3(0.95, 1.05, 0.95)
                         : vec3(0.95, 0.95, 1.05);
    col *= mask;
    // vignette
    vec2 d = v_tex - 0.5;
    col *= 1.0 - dot(d, d) * 0.35;
    gl_FragColor = vec4(col, 1.0);
}
#endif
