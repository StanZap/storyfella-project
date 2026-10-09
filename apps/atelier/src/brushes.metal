#include <metal_stdlib>
using namespace metal;
struct Segment { float ax, ay, bx, by, pa, pb; };

kernel void clear_tile(texture2d<float, access::write> image [[texture(0)]], uint2 p [[thread_position_in_grid]]) {
    if (p.x < image.get_width() && p.y < image.get_height()) image.write(float4(0), p);
}
kernel void paper_tile(texture2d<float, access::write> output [[texture(0)]], constant float *u [[buffer(0)]], uint2 p [[thread_position_in_grid]]) {
    if (p.x < output.get_width() && p.y < output.get_height()) output.write(u[0] > 0 ? float4(0) : float4(1,253.0/255.0,248.0/255.0,1),p);
}
// Each thread owns one coverage pixel. Union footprints before applying pigment to
// the immutable pre-stroke tile: event rate cannot accidentally darken a stroke.
kernel void brush_tile(const device Segment *segments [[buffer(0)]], constant float *u [[buffer(1)]],
    texture2d<float, access::read> base [[texture(0)]],
    texture2d<float, access::read_write> coverage [[texture(1)]],
    texture2d<float, access::write> painted [[texture(2)]], uint2 id [[thread_position_in_grid]]) {
    if (id.x >= uint(u[10]) || id.y >= uint(u[11])) return;
    uint2 p = id + uint2(u[8],u[9]);
    if (p.x >= painted.get_width() || p.y >= painted.get_height()) return;
    float2 position = float2(p) + float2(u[1], u[2]) + 0.5;
    float value = coverage.read(p).r;
    for (uint i = 0; i < uint(u[0]); ++i) {
        Segment s = segments[i];
        float2 a(s.ax,s.ay), b(s.bx,s.by), delta=b-a;
        float t = clamp(dot(position-a,delta) / max(dot(delta,delta),0.00001),0.0,1.0);
        float pressure = mix(s.pa,s.pb,t);
        float radius = max(u[3]*(0.12+0.88*pressure),0.25);
        float distance = length(position-mix(a,b,t));
        float edge = uint(u[5]) == 2 && u[6] == 0 ? sqrt(clamp(1-distance/radius,0.0,1.0)) : clamp(radius+0.5-distance,0.0,1.0);
        float strength = u[6] > 0 ? 1 : 0.15+0.85*pressure;
        value = max(value,edge*strength);
    }
    coverage.write(float4(value),p);
    float alpha = value*u[4];
    if (u[6] == 0) {
        uint hash = uint(position.x)*374761393u ^ uint(position.y)*668265263u;
        hash = (hash ^ (hash>>13))*1274126177u;
        float grain = float(hash & 255u)/255.0;
        switch (uint(u[5])) {
            case 0: alpha *= grain < 0.18 ? 0.12 : 0.3+grain*0.65; break;
            case 2: alpha *= 0.35+grain*0.65; break;
            case 3: alpha *= 0.35; break;
        }
    }
    float4 original = base.read(p);
    painted.write(u[6] > 0 ? original*(1-alpha) : float4(float3(41,37,44)/255.0,1)*alpha+original*(1-alpha),p);
}
kernel void composite_tile(texture2d<float, access::read> layer [[texture(0)]],
    texture2d<float, access::read_write> output [[texture(1)]], constant float *u [[buffer(0)]], uint2 p [[thread_position_in_grid]]) {
    if (p.x >= output.get_width() || p.y >= output.get_height()) return;
    float4 pigment = layer.read(p)*u[0];
    output.write(pigment+output.read(p)*(1-pigment.a),p);
}
kernel void trace_tile(texture2d<float, access::read> reference [[texture(0)]],
    texture2d<float, access::read_write> output [[texture(1)]], constant float *u [[buffer(0)]], uint2 p [[thread_position_in_grid]]) {
    if (p.x >= output.get_width() || p.y >= output.get_height()) return;
    float2 doc = float2(p)+float2(u[0],u[1])+0.5;
    float2 uv = (doc-float2(u[2],u[3]))/u[4];
    if (uv.x < 0 || uv.y < 0 || uv.x >= reference.get_width() || uv.y >= reference.get_height()) return;
    float4 sample = reference.read(uint2(uv));
    float alpha=sample.a*0.18;
    output.write(float4(sample.rgb*alpha,alpha)+output.read(p)*(1-alpha),p);
}

kernel void orient_tile(texture2d<float,access::read> source [[texture(0)]],
    texture2d<float,access::write> output [[texture(1)]],constant float *u [[buffer(0)]],uint2 p [[thread_position_in_grid]]) {
    if (p.x>=output.get_width() || p.y>=output.get_height()) return;
    uint x=u[1]>0 ? output.get_width()-1-p.x : p.x;
    uint2 sample;
    switch (uint(u[0])%4) {
        case 1: sample=uint2(p.y,source.get_height()-1-x);break;
        case 2: sample=uint2(source.get_width()-1-x,source.get_height()-1-p.y);break;
        case 3: sample=uint2(source.get_width()-1-p.y,x);break;
        default: sample=uint2(x,p.y);
    }
    output.write(source.read(sample),p);
}
