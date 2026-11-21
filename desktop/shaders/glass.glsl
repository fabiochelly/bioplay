// bioplay — verre. Floute la vidéo sous les plaques de l'interface (bureau et Android).
// Les rectangles (pixels de l'image vidéo, origine en haut à gauche) arrivent par glsl-shader-opts.
// glass_n = 0 : aucune passe ne s'exécute (//!WHEN), coût nul quand l'interface est masquée.

//!PARAM glass_n
//!TYPE DYNAMIC int
//!MINIMUM 0
//!MAXIMUM 8
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

//!PARAM r4x
//!TYPE DYNAMIC float
0.0

//!PARAM r4y
//!TYPE DYNAMIC float
0.0

//!PARAM r4w
//!TYPE DYNAMIC float
0.0

//!PARAM r4h
//!TYPE DYNAMIC float
0.0

//!PARAM r4r
//!TYPE DYNAMIC float
0.0

//!PARAM r4a
//!TYPE DYNAMIC float
0.0

//!PARAM r5x
//!TYPE DYNAMIC float
0.0

//!PARAM r5y
//!TYPE DYNAMIC float
0.0

//!PARAM r5w
//!TYPE DYNAMIC float
0.0

//!PARAM r5h
//!TYPE DYNAMIC float
0.0

//!PARAM r5r
//!TYPE DYNAMIC float
0.0

//!PARAM r5a
//!TYPE DYNAMIC float
0.0

//!PARAM r6x
//!TYPE DYNAMIC float
0.0

//!PARAM r6y
//!TYPE DYNAMIC float
0.0

//!PARAM r6w
//!TYPE DYNAMIC float
0.0

//!PARAM r6h
//!TYPE DYNAMIC float
0.0

//!PARAM r6r
//!TYPE DYNAMIC float
0.0

//!PARAM r6a
//!TYPE DYNAMIC float
0.0

//!PARAM r7x
//!TYPE DYNAMIC float
0.0

//!PARAM r7y
//!TYPE DYNAMIC float
0.0

//!PARAM r7w
//!TYPE DYNAMIC float
0.0

//!PARAM r7h
//!TYPE DYNAMIC float
0.0

//!PARAM r7r
//!TYPE DYNAMIC float
0.0

//!PARAM r7a
//!TYPE DYNAMIC float
0.0

//!HOOK SCALED
//!BIND HOOKED
//!SAVE GLASS_DOWN
//!WIDTH HOOKED.w 8 /
//!HEIGHT HOOKED.h 8 /
//!WHEN glass_n
//!DESC bioplay glass downsample
vec4 hook() {
    // Échantillonneur brut (HOOKED_raw × HOOKED_mul) : accepté par tous les compilateurs,
    // y compris OpenGL ES sur Android, où HOOKED_tex(pos décalée) est refusé.
    vec2 d = HOOKED_pt * 2.0;
    vec4 c = texture(HOOKED_raw, HOOKED_pos + vec2(-d.x, -d.y)) + texture(HOOKED_raw, HOOKED_pos + vec2(d.x, -d.y))
           + texture(HOOKED_raw, HOOKED_pos + vec2(-d.x, d.y)) + texture(HOOKED_raw, HOOKED_pos + vec2(d.x, d.y));
    return HOOKED_mul * 0.25 * c;
}

//!HOOK SCALED
//!BIND GLASS_DOWN
//!SAVE GLASS_H
//!WIDTH GLASS_DOWN.w
//!HEIGHT GLASS_DOWN.h
//!WHEN glass_n
//!DESC bioplay glass blur h
vec4 hook() {
    float w[5] = float[](0.2270, 0.1945, 0.1216, 0.0540, 0.0162);
    vec4 c = GLASS_DOWN_tex(GLASS_DOWN_pos) * w[0];
    for (int i = 1; i < 5; i++) {
        vec2 o = vec2(GLASS_DOWN_pt.x * float(i) * 2.0, 0.0);
        c += (GLASS_DOWN_tex(GLASS_DOWN_pos + o) + GLASS_DOWN_tex(GLASS_DOWN_pos - o)) * w[i];
    }
    return c;
}

//!HOOK SCALED
//!BIND GLASS_H
//!SAVE GLASS_BLUR
//!WIDTH GLASS_H.w
//!HEIGHT GLASS_H.h
//!WHEN glass_n
//!DESC bioplay glass blur v
vec4 hook() {
    float w[5] = float[](0.2270, 0.1945, 0.1216, 0.0540, 0.0162);
    vec4 c = GLASS_H_tex(GLASS_H_pos) * w[0];
    for (int i = 1; i < 5; i++) {
        vec2 o = vec2(0.0, GLASS_H_pt.y * float(i) * 2.0);
        c += (GLASS_H_tex(GLASS_H_pos + o) + GLASS_H_tex(GLASS_H_pos - o)) * w[i];
    }
    return c;
}

//!HOOK SCALED
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
    // Flou seul : pas d'effet de lentille sur les bords (il dessinait un anneau et des bandes).
    vec3 b = (GLASS_BLUR_mul * texture(GLASS_BLUR_raw, px / HOOKED_size)).rgb;
    float l = dot(b, vec3(0.299, 0.587, 0.114));
    b = mix(vec3(l), b, 1.3);
    float aa = smoothstep(0.5, -0.5, d);
    return vec4(mix(c.rgb, b, aa * alpha), c.a);
}
vec4 hook() {
    vec4 c = HOOKED_mul * texture(HOOKED_raw, HOOKED_pos);
    vec2 px = HOOKED_pos * HOOKED_size;
    if (glass_n > 0) c = plate(c, px, vec4(r0x, r0y, r0w, r0h), r0r, r0a);
    if (glass_n > 1) c = plate(c, px, vec4(r1x, r1y, r1w, r1h), r1r, r1a);
    if (glass_n > 2) c = plate(c, px, vec4(r2x, r2y, r2w, r2h), r2r, r2a);
    if (glass_n > 3) c = plate(c, px, vec4(r3x, r3y, r3w, r3h), r3r, r3a);
    if (glass_n > 4) c = plate(c, px, vec4(r4x, r4y, r4w, r4h), r4r, r4a);
    if (glass_n > 5) c = plate(c, px, vec4(r5x, r5y, r5w, r5h), r5r, r5a);
    if (glass_n > 6) c = plate(c, px, vec4(r6x, r6y, r6w, r6h), r6r, r6a);
    if (glass_n > 7) c = plate(c, px, vec4(r7x, r7y, r7w, r7h), r7r, r7a);
    return c;
}
