Texture2D source : register(t0);
SamplerState linearSampler : register(s0);
cbuffer Frame : register(b0) {
    float2 sourceSize;
    float2 outputSize;
    float2 cursor;
    float visible;
    float cursorSize;
    float opacity;
    float cursorStyle;
    float2 padding;
}
struct Vertex { float4 pos : SV_POSITION; float2 uv : TEXCOORD0; };
Vertex vsMain(uint id : SV_VertexID) {
    Vertex v;
    v.uv = float2((id << 1) & 2, id & 2);
    v.pos = float4(v.uv * float2(2, -2) + float2(-1, 1), 0, 1);
    return v;
}
float cross2(float2 a, float2 b) { return a.x * b.y - a.y * b.x; }
bool insideTriangle(float2 p, float2 a, float2 b, float2 c) {
    float s = cross2(b-a,p-a), t = cross2(c-b,p-b), u = cross2(a-c,p-c);
    return (s >= 0 && t >= 0 && u >= 0) || (s <= 0 && t <= 0 && u <= 0);
}
float4 psMain(Vertex v) : SV_TARGET {
    float scale = min(outputSize.x / sourceSize.x, outputSize.y / sourceSize.y);
    float2 origin = (outputSize - sourceSize * scale) * 0.5;
    float2 pos = (v.pos.xy - origin) / scale;
    float4 color = float4(0.045,0.055,0.065,1);
    if (all(pos >= 0) && all(pos < sourceSize)) color = source.Sample(linearSampler, pos / sourceSize);
    float4 original = color;
    float2 p = (pos - cursor) / cursorSize;
    if (visible > 0.5 && cursorStyle < 0.5) {
        bool outer = insideTriangle(p,float2(-1,-1),float2(21,20),float2(-1,30));
        outer = outer || insideTriangle(p,float2(7,17),float2(13,14),float2(22,32)) || insideTriangle(p,float2(7,17),float2(22,32),float2(16,35));
        bool inner = insideTriangle(p,float2(1,2),float2(17,19),float2(1,25));
        inner = inner || insideTriangle(p,float2(9,18),float2(12,17),float2(19,31)) || insideTriangle(p,float2(9,18),float2(19,31),float2(17,32));
        if (outer) color = float4(0.06,0.09,0.12,1);
        if (inner) color = float4(0.95,0.98,1,1);
    }
    if (visible > 0.5 && cursorStyle >= 0.5) {
        float radius = length(p);
        if (cursorStyle < 1.5) {
            if (radius < 9) color = float4(0.06,0.09,0.12,1);
            if (radius < 6) color = float4(0.95,0.98,1,1);
        } else {
            if (radius > 7 && radius < 13) color = float4(0.06,0.09,0.12,1);
            if (radius > 9 && radius < 11) color = float4(0.95,0.98,1,1);
        }
    }
    return float4(lerp(original.rgb,color.rgb,opacity),1);
}
