pub(in crate::spectrogram) fn grouped_halos() -> bool {
    ACTIVE.with_borrow(|name| name.starts_with("full-grouped-v"))
}

pub(in crate::spectrogram) struct ResearchHaloGroups {
    pub factors: Vec<f32>,
    pub counts: Vec<u32>,
    pub group: [usize; 5],
    pub layer: [u32; 5],
}

pub(in crate::spectrogram) fn grouped_halo_plan() -> ResearchHaloGroups {
    ACTIVE.with_borrow(|name| {
        assert!(name.starts_with("full-grouped-v"));
        let mut result = ResearchHaloGroups {
            factors: Vec::new(), counts: Vec::new(), group: [0; 5], layer: [0; 5],
        };
        for (k, factor) in factors(name).into_iter().enumerate() {
            // Scratch targets are 1x1 at every factor, so grouping by their
            // rounded extent would silently disagree with the shader's map.
            let group = if let Some(group) = result.factors.iter().position(|f| f.to_bits() == factor.to_bits()) {
                group
            } else {
                result.factors.push(factor);
                result.counts.push(0);
                result.factors.len() - 1
            };
            result.group[k] = group;
            result.layer[k] = result.counts[group];
            result.counts[group] += 1;
        }
        assert!(result.factors.len() <= 3, "grouped experiment supports at most three distinct factors");
        result
    })
}
