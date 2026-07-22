// kUI LCD: soft cell grid with slight desaturation, handheld feel.
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
varying vec2 v_tex;
void main() {
    vec3 col = texture2D(Texture, v_tex).rgb;
    vec2 cell = fract(v_tex * TextureSize);
    float gx = smoothstep(0.0, 0.12, cell.x) * smoothstep(1.0, 0.88, cell.x);
    float gy = smoothstep(0.0, 0.12, cell.y) * smoothstep(1.0, 0.88, cell.y);
    col *= 0.78 + 0.22 * gx * gy;
    float grey = dot(col, vec3(0.299, 0.587, 0.114));
    col = mix(vec3(grey), col, 0.92);
    gl_FragColor = vec4(col, 1.0);
}
#endif
