from pathlib import Path
A=Path(__file__).parent; D=A/'shaders'; base=(D/'base.wgsl').read_text()
for n in ['base-a','base-b','full-a','full-b']:(D/f'{n}.wgsl').write_text(base)
split3=base.replace('star_layers(pt, 2u, STAR_SLICES, far)','star_layers(pt, 3u, STAR_SLICES, far)').replace('star_layers(pt, 0u, 2u, palette_color(0.0))','star_layers(pt, 0u, 3u, palette_color(0.0))')
for n in ['s3-a','s3-b']:(D/f'{n}.wgsl').write_text(split3)
# Core support is confined to its own cell; the other eight halo contributors
# cannot contain core coverage. Keep exact original native/core arithmetic.
micro=base.replace('halo: bool) -> vec4<f32>','halo: bool, has_core: bool) -> vec4<f32>')
micro=micro.replace('let core = gaussian * (1.0 - smoothstep(cloud.star_geometry.z * reach, reach, dist));','var core = 0.0;\n    if has_core { core = gaussian * (1.0 - smoothstep(cloud.star_geometry.z * reach, reach, dist)); }')
micro=micro.replace('row - 1, true)','row - 1, true, false)').replace('row, true)','row, true, y == 0)').replace('row + 1, true)','row + 1, true, false)').replace('index, false)','index, false, true)')
(D/'micro.wgsl').write_text(micro)
for n in ['complete','full-complete']:(D/f'{n}.wgsl').write_text((D/'reference-complete.wgsl').read_text())
shape='''fn arch_texel(s: StarSlice, f: vec2<f32>, index: i32, halo: bool, complete: bool, short: bool) -> vec4<f32> {
    let t = textureLoad(star_atlas, vec2<i32>(index & (STAR_ATLAS_WIDTH - 1), index >> STAR_ATLAS_SHIFT), 0);
    if t.w == 0u { return vec4<f32>(0.0); }
    let dist = length(f - vec2<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y))) * s.cell;
    let reach = cloud.star_geometry.y * s.cell;
    let radius = select(STAR_HALO_REACH, 1.0 - cloud.star_geometry.x * 0.5, short);
    let outer = radius * s.cell;
    if dist >= select(reach, outer, halo || complete) {return vec4<f32>(0.0);}
    let colour = vec3<f32>(vec3<u32>(t.z >> 20u, t.z >> 10u, t.z) & vec3<u32>(1023u)) / 1023.0;
    let shape = unpack2x16float(t.w);
    let d = dist * shape.x;
    let gaussian = exp(-0.5 * d * d);
    var core = 0.0;
    if !complete {core = gaussian * (1.0 - smoothstep(cloud.star_geometry.z * reach, reach, dist));}
    var cover = core;
    if halo || complete {
        var full = gaussian;
        if s.fringe > 0.0 {full += s.fringe * exp(-0.4 * d);}
        // Preserve the previous half-jitter shape: fade .7 -> .85 cells at default.
        let fade_start = select(STAR_HALO_FADE * radius, radius - 0.15, short);
        full = min(full, 1.0) * (1.0 - smoothstep(fade_start * s.cell, outer, dist));
        cover = max(full - core, 0.0);
    }
    cover *= shape.y;
    return vec4<f32>(colour * cover, cover);
}
fn arch_gather(s: StarSlice, r: vec2<f32>, short: bool, complete: bool) -> vec4<f32> {
    let o = floor(r - select(vec2<f32>(0.0), vec2<f32>(0.5), short));
    let f = r - o;
    let local = vec2<i32>(o) - vec2<i32>(floor(s.offset)) - s.origin;
    let index = s.base + local.y * s.grid.x + local.x;
    var result = vec4<f32>(0.0);
    if short {
        result += arch_texel(s, f, index, true, complete, true);
        result += arch_texel(s, f - vec2<f32>(1.0,0.0), index+1, true, complete, true);
        result += arch_texel(s, f - vec2<f32>(0.0,1.0), index+s.grid.x, true, complete, true);
        result += arch_texel(s, f - vec2<f32>(1.0,1.0), index+s.grid.x+1, true, complete, true);
    } else {
        for (var y=-1; y<=1; y+=1) {
            let row=index+y*s.grid.x; let fy=f.y-f32(y);
            result += arch_texel(s, vec2<f32>(f.x+1.0,fy), row-1, true, complete, false);
            result += arch_texel(s, vec2<f32>(f.x,fy), row, true, complete, false);
            result += arch_texel(s, vec2<f32>(f.x-1.0,fy), row+1, true, complete, false);
        }
    }
    return result;
}
'''
# The original star_texel remains for native cores, avoiding an unrelated change.
common=base.replace('// The low-resolution target stores',shape+'\n// The low-resolution target stores',1)
a=common.index('    let o = floor(r);',common.index('fn fs_star_halo('));b=common.index('\n}',a)
common=common[:a]+'''    return arch_gather(s,r,(ARCH_SHORT & (1u << in.layer)) != 0u,false);'''+common[b:]
old='''        var slice = star_texel(s, f, index, false);
        slice += star_halo_at(pt, k);'''
new='''        var slice = vec4<f32>(0.0);
        if (ARCH_DIRECT & (1u << k)) != 0u {
            slice = arch_gather(s,r,(ARCH_SHORT & (1u << k)) != 0u,true);
        } else {
            slice = star_texel(s, f, index, false);
            slice += star_halo_at(pt, k);
        }'''
assert old in common;common=common.replace(old,new)
cases={'direct4':(31,31,0,1.),'residual4':(0,31,0,1.)}
for mask in [1,2,3,4,8,12,16,24,28]:cases[f'hybrid-{mask}']=(31^mask,31^mask,0,1.)
for count in [3,5]:
 for pct in [100,75,50]:cases[f'group{count}-{pct}']=((1<<count)-1,0,count,pct/100)
cases['group3-short100']=(7,7,3,1.0)
cases['group3-short75']=(7,7,3,0.75)
cases['group3-short50']=(7,7,3,0.5)
# Keep broad support everywhere to verify our refactored long gather separately.
cases['long-ref']=(0,0,0,1.)
for name,(direct,short,group,scale) in cases.items():
 text=f'const ARCH_DIRECT:u32={direct}u;\nconst ARCH_SHORT:u32={short}u;\n'+common
 if group:
  text=text.replace('let far = textureLoad(cloud_tone, vec2<i32>(pt * cloud.ppp), 0).rgb;', 'let far = textureSampleLevel(cloud_tone, cloud_sampler, pt / cloud.size, 0.0).rgb;')
  text=text.replace('star_layers(pt, 2u, STAR_SLICES, far)',f'star_layers(pt, {group}u, STAR_SLICES, far)')
  text=text.replace('let pt = position / cloud.ppp - cloud.origin;\n    // Layer compositing',f'let pt = in.position.xy / ceil(cloud.size * cloud.ppp * {scale}) * cloud.size;\n    // Layer compositing')
  text=text.replace('star_layers(pt, 0u, 2u, palette_color(0.0))',f'star_layers(pt, 0u, {group}u, palette_color(0.0))')
 (D/f'{name}.wgsl').write_text(text)
print('generated',len(list(D.glob('*.wgsl'))),'shader variants')

(D/'full-residual4.wgsl').write_text((D/'residual4.wgsl').read_text())
r=(D/'direct4.wgsl').read_text().replace('if short {\n        result', 'if false {\n        result').replace('true, complete, false);', 'true, complete, short);')
(D/'direct4-ref9.wgsl').write_text(r)
