// Edge-blend shader. Output signal is split into n equal tiles; each tile shows
// one projector via a wall-space homography (uv → q) with distance-weighted blend.
// Hot-reloaded: save this file and the change is live.
// Do not use [unroll] on the projector loops: it makes D3DCompile take minutes.

cbuffer Params : register(b0) {
    float n;
    float gamma;
    float curve;
    float pattern;       // 0 desktop, 1 diagnostic, 2 alignment, 3 gray, 4 white,
                         // 5 vertical lines, 6 horizontal lines, 7 diagonal lines
    float view;          // 0 = signal/output, 1 = wall light-sum simulation
    float mirror;        // 1 = flip output left/right
    float reverseOrder;  // 1 = box feeds projectors right-to-left
    float blendOn;       // 0 = ramps disabled
    float showCursor;
    float canvasW;
    float canvasH;
    float srcW;
    float srcH;
    float2 cursor;       // wall q
    float tileW;         // output tile width in pixels (for edge-bar thickness)
    float muteMask;      // bit i = projector i blacked out (diagnostics)
    float3 _pad0;
};

bool isMuted(float fi) {
    return fmod(floor(muteMask / exp2(fi) + 1e-3), 2.0) >= 1.0;
}

cbuffer Geometry : register(b1) {
    float4 HF[18];       // 3 rows × up to 6 projectors (xyz = Mat3 row)
    float4 HI[18];
    float4 seams[5];     // seams[k].xy = wall q of overlap centre k|k+1
};

Texture2D frameTex : register(t0);
SamplerState smp : register(s0);

struct VO {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

VO vs_main(uint id : SV_VertexID) {
    VO o;
    float2 uv = float2((id << 1) & 2, id & 2);
    o.uv = uv;
    o.pos = float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
    return o;
}

float2 applyH(float4 rows[3], float2 p) {
    float3 h = float3(
        rows[0].x * p.x + rows[0].y * p.y + rows[0].z,
        rows[1].x * p.x + rows[1].y * p.y + rows[1].z,
        rows[2].x * p.x + rows[2].y * p.y + rows[2].z
    );
    return h.xy / max(h.z, 1e-8);
}

float2 applyHF(int fi, float2 uv) {
    int o = fi * 3;
    float4 rows[3] = { HF[o], HF[o + 1], HF[o + 2] };
    return applyH(rows, uv);
}

float2 applyHI(int fi, float2 q) {
    int o = fi * 3;
    float4 rows[3] = { HI[o], HI[o + 1], HI[o + 2] };
    return applyH(rows, q);
}

bool inside01(float2 p) {
    return p.x >= 0 && p.x <= 1 && p.y >= 0 && p.y <= 1;
}

// Pure RGB by projector index (repeats).
float3 projColor(float idx) {
    float k = fmod(idx, 3.0);
    return k < 0.5 ? float3(1, 0, 0)
         : (k < 1.5 ? float3(0, 1, 0) : float3(0, 0, 1));
}

// ---- Seven-segment digits (cell H tall, 0.6H wide, stroke ~H/9) ----
static const int SEG[10] = { 0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F };

bool hitSegH(float2 q, float y, float x0, float x1, float ht) {
    return q.y >= y - ht && q.y <= y + ht && q.x >= x0 && q.x <= x1;
}

bool hitSegV(float2 q, float x, float y0, float y1, float ht) {
    return q.x >= x - ht && q.x <= x + ht && q.y >= y0 && q.y <= y1;
}

bool digitAt(float2 p, float2 o, float H, int d) {
    float2 q = p - o;
    float W = 0.6 * H;
    float t = H / 9.0;
    float ht = t * 0.5;
    if (q.x < -t || q.x > W + t || q.y < -t || q.y > H + t) return false;
    int m = SEG[clamp(d, 0, 9)];
    float y0 = 0, y1 = H * 0.5, y2 = H;
    if ((m & 1)  && hitSegH(q, y0, t, W - t, ht)) return true;
    if ((m & 64) && hitSegH(q, y1, t, W - t, ht)) return true;
    if ((m & 8)  && hitSegH(q, y2, t, W - t, ht)) return true;
    if ((m & 32) && hitSegV(q, 0, t, y1 - t, ht)) return true;
    if ((m & 2)  && hitSegV(q, W, t, y1 - t, ht)) return true;
    if ((m & 16) && hitSegV(q, 0, y1 + t, y2 - t, ht)) return true;
    if ((m & 4)  && hitSegV(q, W, y1 + t, y2 - t, ht)) return true;
    return false;
}

bool numberAt(float2 p, float2 o, float H, uint value) {
    uint v = value;
    int nd = 1;
    [loop] for (int c = 0; c < 4; c++) {
        if (v >= 10) { v /= 10; nd++; }
    }
    float W = 0.6 * H;
    float cell = W + W * 0.25;
    uint place = 1;
    [loop] for (int s = 1; s < 5; s++) {
        if (s < nd) place *= 10;
    }
    uint rem = value;
    [loop] for (int d = 0; d < 5; d++) {
        if (d < nd) {
            int dig = (int)(rem / place);
            rem -= (uint)dig * place;
            place = max(place / 10, 1);
            if (digitAt(p, o + float2(d * cell, 0), H, dig)) return true;
        }
    }
    return false;
}

float3 alignmentPattern(float2 sp, float fi, float s) {
    float3 col = projColor(fi) * 0.35;

    float gThin = 100.0 * s;
    float gThick = 500.0 * s;
    float2 g = abs(frac(sp / gThin + 0.5) - 0.5) * gThin;
    float2 G = abs(frac(sp / gThick + 0.5) - 0.5) * gThick;
    if (min(g.x, g.y) < 1.0 * s || min(G.x, G.y) < 3.0 * s) col = float3(1, 1, 1);

    float2 c = abs(sp - float2(canvasW, canvasH) * 0.5);
    if ((c.x < 4 * s && c.y < 60 * s) || (c.y < 4 * s && c.x < 60 * s)) col = float3(1, 1, 0);

    float2 cen = applyHF((int)fi, float2(0.5, 0.25)) * float2(canvasW, canvasH);
    [loop] for (int k = 0; k < 6; k++) {
        if (k <= (int)fi) {
            float2 dc = float2(cen.x + (k - fi * 0.5) * 70.0 * s, cen.y);
            float r = length(sp - dc);
            if (r < 26 * s) col = projColor(fi);
            if (r < 20 * s) col = float3(1, 1, 1);
        }
    }
    return col;
}

// Thin white lines on black, drawn in wall space so they run continuously across
// projectors. pw = wall pixels per output pixel; lines are ~2 output pixels wide.
float3 linesPattern(float2 sp, float pw, float s, float mode) {
    float pitch = 100.0 * s;
    float2 g = abs(frac(sp / pitch + 0.5) - 0.5) * pitch;
    float d;
    if (mode < 0.5)       d = g.x;
    else if (mode < 1.5)  d = g.y;
    else {
        float2 r = float2(sp.x + sp.y, sp.x - sp.y) * 0.70710678;
        float2 gd = abs(frac(r / pitch + 0.5) - 0.5) * pitch;
        d = min(gd.x, gd.y);
    }
    float a = saturate((pw - d) / pw + 0.5);
    return float3(a, a, a);
}

float3 diagnosticPattern(float2 sp, float2 q, float2 uv, float fi, float s) {
    float3 P = projColor(fi);
    float3 c = float3(0, 0, 0);

    // Soft tint where a neighbour also covers this wall point.
    if (fi > 0) {
        float2 uvn = applyHI((int)fi - 1, q);
        if (inside01(uvn)) c = P * 0.12;
    }
    if (fi < n - 1) {
        float2 uvn = applyHI((int)fi + 1, q);
        if (inside01(uvn)) c = P * 0.12;
    }

    float g50 = 50.0 * s;
    float g250 = 250.0 * s;
    float d50 = min(abs(sp.x - round(sp.x / g50) * g50),
                    abs(sp.y - round(sp.y / g50) * g50));
    float d250 = min(abs(sp.x - round(sp.x / g250) * g250),
                     abs(sp.y - round(sp.y / g250) * g250));
    if (d50 < 1.0 * s) c = P * 0.55;
    if (d250 < 2.0 * s) c = P;

    float vx = floor(sp.x / g250) * g250;
    float digH = 28.0 * s;
    if (numberAt(sp, float2(vx + 6.0 * s, 40.0 * s), digH, (uint)round(vx)) ||
        numberAt(sp, float2(vx + 6.0 * s, canvasH - 80.0 * s), digH, (uint)round(vx)))
        c = P;

    float vy = floor(sp.y / g250) * g250;
    float leftX = applyHF((int)fi, float2(0.0, 0.5)).x * canvasW + 30.0 * s;
    if (numberAt(sp, float2(leftX, vy + 6.0 * s), digH, (uint)round(vy)))
        c = P;

    {
        float H = 180.0 * s;
        float W = 0.6 * H;
        float2 cen = applyHF((int)fi, float2(0.5, 0.5)) * float2(canvasW, canvasH);
        float2 o = float2(cen.x - W * 0.5, cen.y - H * 0.5);
        if (digitAt(sp, o, H, (int)fi + 1)) c = P;
    }

    [loop] for (int k = 0; k < 5; k++) {
        if (k < (int)n - 1) {
            float2 cen = seams[k].xy * float2(canvasW, canvasH);
            float2 d = sp - cen;
            float r = length(d);
            if (abs(r - 40 * s) < 1.5 * s || abs(r - 90 * s) < 1.5 * s || abs(r - 140 * s) < 1.5 * s) c = P;
            if ((abs(d.x) < 1.5 * s && abs(d.y) < 160 * s) || (abs(d.y) < 1.5 * s && abs(d.x) < 160 * s)) c = P;
        }
    }

    float e = 6.0 / max(tileW, 1.0);
    if (uv.x < e || uv.x > 1.0 - e || uv.y < e || uv.y > 1.0 - e) c = P;

    return c;
}

float blendWeight(float fi, float2 q) {
    float sum = 0;
    float a_fi = 0;
    [loop] for (int j = 0; j < 6; j++) {
        if (j < (int)n) {
            float2 uvj = applyHI(j, q);
            float a = 0;
            if (inside01(uvj))
                a = max(pow(saturate(min(uvj.x, 1.0 - uvj.x)), curve), 1e-6);
            sum += a;
            if (j == (int)fi) a_fi = a;
        }
    }
    return a_fi / max(sum, 1e-6);
}

float3 shade(float fi, float2 uv) {
    // Muted = as if that projector were switched off; others keep their blend weights.
    if (isMuted(fi)) return float3(0, 0, 0);
    float2 q = applyHF((int)fi, uv);
    if (!inside01(q)) return float3(0, 0, 0);

    float2 sp = q * float2(canvasW, canvasH);
    float s = canvasH / 1080.0;
    bool isDiag = pattern > 0.5 && pattern < 1.5;

    float3 c;
    if (pattern < 0.5)      c = frameTex.Sample(smp, q).rgb; // source stretched over the wall
    else if (isDiag)        c = diagnosticPattern(sp, q, uv, fi, s);
    else if (pattern < 2.5) c = alignmentPattern(sp, fi, s);
    else if (pattern < 3.5) c = float3(0.5, 0.5, 0.5);
    else if (pattern < 4.5) c = float3(1, 1, 1);
    else {
        // Wall pixels per output pixel, measured by one-pixel step in tile uv.
        float2 spx = applyHF((int)fi, uv + float2(1.0 / max(tileW, 1.0), 0)) * float2(canvasW, canvasH);
        float pw = max(length(spx - sp), 1e-3);
        c = linesPattern(sp, pw, s, pattern - 5.0 + 0.01);
    }

    if (showCursor > 0.5) {
        float2 d = sp - cursor * float2(canvasW, canvasH);
        if (d.y >= -1.5 * s && d.y < 21.5 * s && d.x >= -1.5 * s && d.x <= d.y * 0.55 + 1.5 * s) c = 0;
        if (d.y >= 1.0 * s && d.y < 19.0 * s && d.x >= 0.0 && d.x <= d.y * 0.55 - 0.8 * s) c = 1;
    }

    if (!isDiag && blendOn > 0.5) {
        float w = blendWeight(fi, q);
        c *= pow(w, 1.0 / gamma);
    }

    // Alignment: mark projector edges unblended.
    if (pattern > 1.5 && pattern < 2.5) {
        float e = 4.0 / max(tileW, 1.0);
        if (uv.x < e || uv.x > 1.0 - e || uv.y < e || uv.y > 1.0 - e) c = projColor(fi);
    }

    return c;
}

float4 ps_main(VO i) : SV_Target {
    if (view >= 0.5) {
        // Wall simulation: preview uv is wall q directly.
        float2 q = i.uv;
        float3 light = 0;
        bool isDiag = pattern > 0.5 && pattern < 1.5;
        [loop] for (int j = 0; j < 6; j++) {
            if (j < (int)n) {
                float2 uvj = applyHI(j, q);
                if (inside01(uvj)) {
                    float3 col = shade((float)j, uvj);
                    if (isDiag)
                        light += saturate(col); // unblended colours add (yellow/cyan)
                    else
                        light += pow(saturate(col), gamma);
                }
            }
        }
        if (isDiag)
            return float4(saturate(light), 1);
        return float4(pow(saturate(light), 1.0 / gamma), 1);
    }

    float ux = mirror > 0.5 ? 1.0 - i.uv.x : i.uv.x;
    float tile = min(floor(ux * n), n - 1);
    float fi = reverseOrder > 0.5 ? n - 1 - tile : tile;
    float2 uv = float2(ux * n - tile, i.uv.y);
    return float4(shade(fi, uv), 1);
}
