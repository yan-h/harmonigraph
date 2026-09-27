fn complete_mode(name: &str) -> bool {
    name.starts_with("full-complete") || name.starts_with("full-grouped-complete-v")
        || name.starts_with("full-separate-complete-v")
}

fn complete_source(mut source: String, name: &str) -> String {
    assert_eq!(compute_mode(), 0, "complete response prototype is fragment-only");
    assert!(!fused() && u8_range(name).is_none());
    let native = factors(name).iter().map(|&f| if f == 1.0 { "true" } else { "false" }).collect::<Vec<_>>().join(",");
    let mut helper = std::fs::read_to_string(
        "/private/tmp/stars-investigation/complete-proposal/complete.wgsl"
    ).unwrap();
    assert_eq!(helper.matches("__COMPLETE_DEPTHS__").count(), 1);
    helper = helper.replace("__COMPLETE_DEPTHS__", &native);
    let load = if grouped_halos() {
        r#"let layer = RESEARCH_HALO_LAYERS[k];
    switch RESEARCH_HALO_GROUPS[k] {
        case 0u: { return textureLoad(research_halo_group_0, texel, layer, 0); }
        case 1u: { return textureLoad(research_halo_group_1, texel, layer, 0); }
        default: { return textureLoad(research_halo_group_2, texel, layer, 0); }
    }"#.to_owned()
    } else if separate_halos() {
        r#"switch k {
        case 0u: { return textureLoad(research_halo_0, texel, 0); }
        case 1u: { return textureLoad(research_halo_1, texel, 0); }
        case 2u: { return textureLoad(research_halo_2, texel, 0); }
        case 3u: { return textureLoad(research_halo_3, texel, 0); }
        default: { return textureLoad(research_halo_4, texel, 0); }
    }"#.to_owned()
    } else {
        "return textureLoad(star_halos, texel, i32(k), 0);".to_owned()
    };
    assert_eq!(helper.matches("__LOAD_RESPONSE__").count(), 1);
    helper = helper.replace("__LOAD_RESPONSE__", &load);

    // Reuse the original halo pass coordinates, index, draw, and nine-add order.
    // Only the native branch chooses a complete response with no core taper.
    let halo = "    var halo = vec4<f32>(0.0);\n";
    assert_eq!(source.matches(halo).count(), 1, "halo accumulator moved");
    let walk = std::fs::read_to_string(
        "/private/tmp/stars-investigation/complete-proposal/complete_walk.wgsl"
    ).unwrap();
    source = source.replace(halo, &format!("{halo}{walk}"));

    // Keep the already-rewritten reduced-layer sample expression verbatim.
    // Move all native-core geometry/loading inside the fallback arm so complete
    // depths really skip it, rather than merely ignoring its result afterward.
    let layers = source.find("fn star_layers(").expect("star_layers");
    let begin = layers + source[layers..].find("        let s = cloud.star_slices[k];").expect("slice geometry");
    let end = begin + source[begin..].find("        if slice.w > 0.0 {").expect("slice composition");
    let fallback = &source[begin..end];
    let old = "var slice = star_texel(s, f, index, false);";
    assert_eq!(fallback.matches(old).count(), 1);
    let read = if grouped_halos() { "research_grouped_halo(" }
        else if separate_halos() { "research_separate_halo(" }
        else { "textureSampleLevel(star_halos" };
    assert_eq!(fallback.matches(read).count(), 1);
    let fallback = fallback.replace(old, "slice = star_texel(s, f, index, false);");
    let replacement = format!(r#"        var slice = vec4<f32>(0.0);
        if research_complete_layer(k) {{
            slice = research_complete_load(pt, k);
        }} else {{
{fallback}        }}
"#);
    source.replace_range(begin..end, &replacement);
    source.push_str(&helper);
    source
}
