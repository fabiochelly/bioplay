// bioplay — verre liquide. Floute et réfracte la vidéo sous les plaques de l'interface.
// Les rectangles (en pixels de l'image, origine en haut à gauche) arrivent par glsl-shader-opts.
// glass_n = 0 : aucune passe ne s'exécute (//!WHEN), coût nul quand l'interface est masquée.

//!PARAM glass_n
//!TYPE DYNAMIC int
//!MINIMUM 0
//!MAXIMUM 4
0

//!PARAM r0x
//!TYPE DYNAMIC float
0.0

//!PARAM r0y
//!TYPE DYNAMIC float
0.0

//!PARAM r0w
//!TYPE DYNAMIC float
0.0

//!PARAM r0h
//!TYPE DYNAMIC float
0.0

//!PARAM r0r
//!TYPE DYNAMIC float
0.0

//!PARAM r0a
//!TYPE DYNAMIC float
0.0

//!PARAM r1x
//!TYPE DYNAMIC float
0.0

//!PARAM r1y
//!TYPE DYNAMIC float
0.0

//!PARAM r1w
//!TYPE DYNAMIC float
0.0

//!PARAM r1h
//!TYPE DYNAMIC float
0.0

//!PARAM r1r
//!TYPE DYNAMIC float
0.0

//!PARAM r1a
//!TYPE DYNAMIC float
0.0

//!PARAM r2x
//!TYPE DYNAMIC float
0.0

//!PARAM r2y
//!TYPE DYNAMIC float
0.0

//!PARAM r2w
//!TYPE DYNAMIC float
0.0

//!PARAM r2h
//!TYPE DYNAMIC float
0.0

//!PARAM r2r
//!TYPE DYNAMIC float
0.0

//!PARAM r2a
//!TYPE DYNAMIC float
0.0

//!PARAM r3x
//!TYPE DYNAMIC float
0.0

//!PARAM r3y
//!TYPE DYNAMIC float
0.0

//!PARAM r3w
//!TYPE DYNAMIC float
0.0

//!PARAM r3h
//!TYPE DYNAMIC float
0.0

//!PARAM r3r
//!TYPE DYNAMIC float
0.0

//!PARAM r3a
//!TYPE DYNAMIC float
0.0

//!HOOK OUTPUT
//!BIND HOOKED
//!SAVE GLASS_DOWN
//!WIDTH HOOKED.w 8 /
//!HEIGHT HOOKED.h 8 /
//!WHEN glass_n
//!DESC bioplay glass downsample
vec4 hook() {
    vec2 d = HOOKED_pt * 2.0;
    return 0.25 * (HOOKED_tex(HOOKED_pos + vec2(-d.x, -d.y)) + HOOKED_tex(HOOKED_pos + vec2(d.x, -d.y))
                 + HOOKED_tex(HOOKED_pos + vec2(-d.x, d.y)) + HOOKED_tex(HOOKED_pos + vec2(d.x, d.y)));
}

//!HOOK OUTPUT
//!BIND GLASS_DOWN
//!SAVE GLASS_H
//!WIDTH GLASS_DOWN.w
//!HEIGHT GLASS_DOWN.h
//!WHEN glass_n
//!DESC bioplay glass blur h
vec4 hook() {
    const float w[5] = float[](0.2270, 0.1945, 0.1216, 0.0540, 0.0162);
    vec4 c = GLASS_DOWN_tex(GLASS_DOWN_pos) * w[0];
    for (int i = 1; i < 5; i++) {
        vec2 o = vec2(GLASS_DOWN_pt.x * float(i) * 2.0, 0.0);
        c += (GLASS_DOWN_tex(GLASS_DOWN_pos + o) + GLASS_DOWN_tex(GLASS_DOWN_pos - o)) * w[i];
    }
    return c;
}

//!HOOK OUTPUT
//!BIND GLASS_H
//!SAVE GLASS_BLUR
//!WIDTH GLASS_H.w
//!HEIGHT GLASS_H.h
//!WHEN glass_n
//!DESC bioplay glass blur v
vec4 hook() {
    const float w[5] = float[](0.2270, 0.1945, 0.1216, 0.0540, 0.0162);
    vec4 c = GLASS_H_tex(GLASS_H_pos) * w[0];
    for (int i = 1; i < 5; i++) {
        vec2 o = vec2(0.0, GLASS_H_pt.y * float(i) * 2.0);
        c += (GLASS_H_tex(GLASS_H_pos + o) + GLASS_H_tex(GLASS_H_pos - o)) * w[i];
    }
    return c;
}

//!HOOK OUTPUT
//!BIND HOOKED
//!BIND GLASS_BLUR
//!WHEN glass_n
//!DESC bioplay glass
float sd(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
vec4 plate(vec4 c, vec2 px, vec4 rect, float radius, float alpha) {
    if (alpha <= 0.0) return c;
    vec2 half_size = rect.zw * 0.5;
    vec2 p = px - (rect.xy + half_size);
    float d = sd(p, half_size, min(radius, min(half_size.x, half_size.y)));
    if (d > 0.5) return c;
    float e = -d;
    // Flou seul : pas d'effet de lentille sur les bords (il dessinait un anneau et des bandes).
    vec3 b = GLASS_BLUR_tex(px / HOOKED_size).rgb;
    float l = dot(b, vec3(0.299, 0.587, 0.114));
    b = mix(vec3(l), b, 1.3);
    float aa = smoothstep(-0.5, 0.5, e);
    return vec4(mix(c.rgb, b, aa * alpha), c.a);
}
vec4 hook() {
    vec4 c = HOOKED_texOff(0);
    vec2 px = HOOKED_pos * HOOKED_size;
    if (glass_n > 0) c = plate(c, px, vec4(r0x, r0y, r0w, r0h), r0r, r0a);
    if (glass_n > 1) c = plate(c, px, vec4(r1x, r1y, r1w, r1h), r1r, r1a);
    if (glass_n > 2) c = plate(c, px, vec4(r2x, r2y, r2w, r2h), r2r, r2a);
    if (glass_n > 3) c = plate(c, px, vec4(r3x, r3y, r3w, r3h), r3r, r3a);
    return c;
}
